# CLI Reference

`graphify-rs` is an AI-powered knowledge graph builder that transforms code, docs, papers, and images into queryable, interactive knowledge graphs.

## Table of Contents

- [Global Flags](#global-flags)
- [Commands](#commands)
  - [build](#graphify-rs-build) — Build knowledge graph
  - [query](#graphify-rs-query) — Query the graph
  - [diff](#graphify-rs-diff) — Compare two graph snapshots
  - [stats](#graphify-rs-stats) — Show graph statistics
  - [watch](#graphify-rs-watch) — Auto-rebuild on file changes
  - [serve](#graphify-rs-serve) — Start MCP server (16 tools)
  - [ingest](#graphify-rs-ingest) — Fetch URL content
  - [hook](#graphify-rs-hook) — Git hook management
  - [install](#graphify-rs-install) — Install skill for AI agents
  - [init](#graphify-rs-init) — Create config file
  - [completions](#graphify-rs-completions) — Shell completions
  - [benchmark](#graphify-rs-benchmark) — Token efficiency
- [Configuration](#configuration-graphifytoml)
- [Agent Integration](#agent-integration)

## Global Flags

These flags can be used with **any** subcommand.

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--quiet` | `-q` | `bool` | `false` | Suppress non-essential output. Only errors are printed. |
| `--verbose` | `-v` | `bool` | `false` | Enable verbose output (debug-level). Sets log filter to `debug`. |
| `--jobs <N>` | `-j` | `usize` | Number of CPUs | Number of parallel jobs. Controls rayon thread pool size and semantic extraction concurrency. |

```bash
graphify-rs -q build                    # silent build
graphify-rs -v build                    # debug output
graphify-rs -j 4 build                  # limit to 4 threads
graphify-rs -q -j 2 serve               # quiet mode, 2 threads
```

---

## Commands

### `graphify-rs build`

Build the knowledge graph from files in a directory. This is the main pipeline: detect files -> extract AST (pass 1) -> semantic extraction via Claude API (pass 2) -> build graph -> cluster communities -> analyze -> export.

#### Parameters

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--path <PATH>` | `-p` | `String` | `"."` | Root directory to scan for source files. |
| `--output <DIR>` | `-o` | `String` | `".graphify"` | Output directory for all generated files. |
| `--no-llm` | | `bool` | `false` | Disable legacy LLM extraction. Local AST + document context extraction still runs. |
| `--code-only` | | `bool` | `false` | Only process code files, skip docs and papers. |
| `--update` | | `bool` | `false` | Safe incremental rebuild: scan the full current file set, but reuse SHA256 extraction cache for unchanged files so `graph.json` stays complete. |
| `--format <FMT,...>` | | `String` (comma-separated) | all formats | Export formats to generate. Available: `json`, `html`, `graphml`, `cypher`, `svg`, `wiki`, `obsidian`, `report`, `context`. |
| `--max-viz-nodes <N>` | | `usize` | `2000` | Maximum nodes in HTML visualization. Larger values show more detail but may slow the browser. |
| `--embed` | | `bool` | `false` | Build `.graphify/semantic-index.json` for semantic graph search. `graphifyq ensure/query` enables this by default. |
| `--embedding-provider <PROVIDER>` | | `String` | `model2vec` | Embedding backend: `model2vec`, `ollama`, or `voyage`. |
| `--embedding-model <MODEL>` | | `String` | provider default | Model2Vec HF ID/local path, Ollama model name, or Voyage model name. Prefixes like `ollama:embeddinggemma` are accepted. |
| `--anthropic-semantic` | | `bool` | `false` | Explicitly enable legacy Anthropic document concept extraction. Requires `ANTHROPIC_API_KEY`. |

#### Examples

```bash
# Full build of current directory, all export formats
graphify-rs build

# Build a specific project, output to custom dir
graphify-rs build --path /path/to/project --output my-graph

# Fast AST-only build (no Claude API calls)
graphify-rs build --no-llm

# Fast local-only build plus local semantic query index and LLM_CONTEXT.md
graphify-rs build --no-llm --embed

# Local Ollama embeddings instead of Model2Vec
ollama pull embeddinggemma
graphify-rs build --no-llm --embed --embedding-provider ollama --embedding-model embeddinggemma

# Hosted Voyage embeddings
VOYAGE_API_KEY=... graphify-rs build --no-llm --embed --embedding-provider voyage --embedding-model voyage-code-3

# Only code files, skip docs/papers
graphify-rs build --code-only

# Incremental rebuild after editing a few files
graphify-rs build --update

# Only generate JSON and HTML
graphify-rs build --format json,html

# Only generate the report
graphify-rs build --format report

# Combine: fast incremental, code-only, JSON+report
graphify-rs build --update --code-only --no-llm --format json,report
```

#### Build Pipeline

1. **Detect** — Scans `--path` for code, doc, paper, and image files (respects root `.gitignore`, `.git/info/exclude`, and `.graphifyignore`, skips sensitive files).
2. **Extract AST (Pass 1)** — Deterministic tree-sitter + regex extraction for code files. Per-file SHA256 cache in `<output>/cache/`.
3. **Local Document Context (Pass 1b)** — Markdown/RST/text headings and prose become concept nodes. This is LLM-free and runs unless `--code-only` is set.
4. **Optional Legacy Anthropic Extraction** — Only runs with `--anthropic-semantic` and `ANTHROPIC_API_KEY`; skipped by default.
5. **Build Graph** — Assemble nodes and edges, deduplicate, and annotate source quality (`source`, `generated`, `minified`, `test`, `build_artifact`, `dependency`, `project_context`).
6. **Cluster** — Leiden community detection + cohesion scoring.
7. **Analyze** — God nodes, surprising connections, suggested questions with generated/minified/test/build artifacts downranked.
8. **Export** — Write selected formats to `--output`, including compact `LLM_CONTEXT.md` by default.

---

### `graphify-rs query`

Query the knowledge graph using natural language. Returns a subgraph context as text.

#### Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `<QUESTION>` (positional) | `String` | *required* | The natural language question to query. |
| `--dfs` | `bool` | `false` | Use depth-first search instead of breadth-first search for traversal. |
| `--budget <N>` | `usize` | `2000` | Maximum token budget for the output text. |
| `--graph <PATH>` | `String` | `".graphify/graph.json"` | Path to the graph JSON file. |

#### Examples

```bash
# Basic query
graphify-rs query "how does authentication work?"

# DFS traversal with larger budget
graphify-rs query "error handling flow" --dfs --budget 3000

# Query a specific graph file
graphify-rs query "database connections" --graph /path/to/graph.json
```

---

### `graphify-rs diff`

Compare two graph snapshots and display the differences (added/removed nodes and edges).

#### Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `<OLD>` (positional) | `String` | *required* | Path to the old `graph.json`. |
| `<NEW>` (positional) | `String` | *required* | Path to the new `graph.json`. |
| `--output <FORMAT>` | `String` | `"text"` | Output format: `text` (colored terminal) or `json`. |

#### Examples

```bash
# Compare two graph versions (colored text output)
graphify-rs diff old-graph/graph.json new-graph/graph.json

# Output as JSON for programmatic use
graphify-rs diff v1/graph.json v2/graph.json --output json
```

---

### `graphify-rs stats`

Show graph statistics without rebuilding. Displays node/edge counts, communities, degree distribution, node types, edge relations, and top connected nodes.

#### Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `<GRAPH>` (positional) | `String` | `".graphify/graph.json"` | Path to the graph JSON file. |

#### Examples

```bash
# Stats for default graph
graphify-rs stats

# Stats for a specific graph file
graphify-rs stats /path/to/graph.json
```

---

### `graphify-rs watch`

Watch a directory for file changes and automatically rebuild the graph incrementally.

#### Parameters

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--path <PATH>` | `-p` | `String` | `"."` | Directory to watch for changes. |
| `--output <DIR>` | `-o` | `String` | `".graphify"` | Output directory for graph files. |

#### Examples

```bash
# Watch current directory
graphify-rs watch

# Watch a specific directory
graphify-rs watch --path src --output my-graph
```

---

### `graphify-rs serve`

Start the MCP (Model Context Protocol) server over JSON-RPC 2.0. Stdio remains the default; local HTTP is available for Codex and short-lived helper clients.

#### Parameters

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--graph <PATH>` | `String` | `".graphify/graph.json"` | Path to the graph JSON file to serve. |
| `--transport <stdio\|http>` | `String` | `"stdio"` | MCP transport. |
| `--http-bind <ADDR>` | `String` | `"127.0.0.1:0"` | HTTP bind address when `--transport=http`. |
| `--http-path <PATH>` | `String` | `"/mcp"` | HTTP MCP endpoint path. |
| `--registry-path <PATH>` | `String` | unset | Optional JSON registry written after binding; used by `graphifyq`. |

#### Available MCP Tools

| Tool | Description |
|------|-------------|
| `query_graph` | Search nodes by keywords or the optional semantic index, return subgraph context |
| `get_node` | Get detailed info about a specific node |
| `get_neighbors` | Get a node's neighbors and connecting edges |
| `get_community` | List all nodes in a community |
| `god_nodes` | Find the most-connected hub nodes |
| `graph_stats` | Overall graph statistics |
| `shortest_path` | Find shortest path between two nodes |
| `find_all_paths` | Enumerate all simple paths between two nodes (DFS, max 50) |
| `weighted_path` | Dijkstra shortest path using edge weights (1/weight distance) |
| `community_bridges` | Find top-N cross-community bridge nodes by bridge ratio |
| `graph_diff` | Compare two graph snapshots and return added/removed nodes and edges |
| `pagerank` | Compute PageRank importance scores (identifies structurally critical nodes) |
| `detect_cycles` | Detect dependency cycles using Tarjan's SCC algorithm |
| `smart_summary` | Multi-level graph summary (detailed / community / architecture) |
| `semantic_query` | Return ranked Model2Vec semantic node matches when `.graphify/semantic-index.json` exists |
| `find_similar` | Find structurally similar node pairs via graph embeddings |

#### Examples

```bash
# Start MCP server with default graph
graphify-rs serve

# Serve a specific graph
graphify-rs serve --graph /path/to/graph.json

# Serve over local HTTP and write a graphifyq registry
graphify-rs serve --transport http --http-bind 127.0.0.1:0 --registry-path .graphify/.graphifyq-server.json --graph .graphify/graph.json
```

---

### `graphifyq`

Short-lived query helper that manages a per-project local HTTP MCP sidecar, similar to `fffq`.

```bash
graphifyq ensure                         # semantic index is built by default; stale graphs auto-refresh every 300s
graphifyq ensure --no-embed              # opt out for strict AST-only/offline startup
graphifyq ensure --no-auto-refresh       # read-only sidecar startup; never rebuild an existing graph
graphifyq ensure --refresh-interval-secs 60
graphifyq query "how does auth work?"     # semantic query context by default
graphifyq query --no-embed "where is queue backpressure handled?"
graphifyq stats
graphifyq summary architecture --budget 3000
graphifyq tool pagerank '{"top_n": 20}'
```

`graphifyq` records its per-repo refresh state in `.graphify/.graphifyq-refresh.json`.
When the interval expires it runs `graphify-rs build --path . --output .graphify --no-llm --update`
and includes `--embed` when semantic search is enabled (the default for `ensure` and `query`).
If a local HTTP sidecar is already running, graphifyq terminates and restarts it after refresh
so subsequent MCP/query calls load the new graph.

---

### `graphify-rs ingest`

Ingest content from a URL (arXiv papers, tweets, PDFs, webpages) and add it to the graph output directory.

#### Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `<URL>` (positional) | `String` | *required* | URL to ingest content from. |
| `--output <DIR>` | `-o` | `String` | `".graphify"` | Output directory. |

#### Examples

```bash
# Ingest an arXiv paper
graphify-rs ingest https://arxiv.org/abs/2301.00001

# Ingest a webpage to custom output
graphify-rs ingest https://example.com/docs --output my-graph
```

---

### `graphify-rs hook`

Git hook management. Install, uninstall, or check status of git hooks that automatically rebuild the graph on commit.

#### Subcommands

| Subcommand | Description |
|------------|-------------|
| `install` | Install git hooks (pre-commit). |
| `uninstall` | Remove installed git hooks. |
| `status` | Show current hook installation status. |

#### Examples

```bash
graphify-rs hook install      # install pre-commit hook
graphify-rs hook uninstall    # remove hooks
graphify-rs hook status       # check if hooks are installed
```

---

### `graphify-rs claude install` / `uninstall`

Project-level Claude Code integration. Installs a `PreToolUse` hook and adds graph instructions to `CLAUDE.md`.

#### What `install` does

1. Appends a `## graphify` section to `./CLAUDE.md` with rules for the agent to read the graph report.
2. Writes a `PreToolUse` hook to `.claude/settings.json` that triggers on `Glob|Grep` tool calls.

#### What `uninstall` does

1. Removes the `## graphify` section from `./CLAUDE.md`.
2. Removes the hook from `.claude/settings.json`.

#### Examples

```bash
graphify-rs claude install
graphify-rs claude uninstall
```

---

### `graphify-rs codex install` / `uninstall`

Project-level Codex integration. Writes hook to `.codex/hooks.json` and adds instructions to `AGENTS.md`.

#### Examples

```bash
graphify-rs codex install
graphify-rs codex uninstall
```

---

### `graphify-rs opencode install` / `uninstall`

Project-level OpenCode integration. Writes a plugin to `.opencode/plugins/graphify.js`, registers it in `opencode.json`, and adds instructions to `AGENTS.md`.

#### Examples

```bash
graphify-rs opencode install
graphify-rs opencode uninstall
```

---

### `graphify-rs codebuddy install` / `uninstall`

Project-level CodeBuddy integration. Writes a `PreToolUse` hook to `.codebuddy/settings.json` and adds instructions to `AGENTS.md`.

#### Examples

```bash
graphify-rs codebuddy install
graphify-rs codebuddy uninstall
```

---

### `graphify-rs claw install` / `uninstall`

Project-level OpenClaw integration. Adds graph instructions to `AGENTS.md`.

#### Examples

```bash
graphify-rs claw install
graphify-rs claw uninstall
```

---

### `graphify-rs droid install` / `uninstall`

Project-level Factory Droid integration. Adds graph instructions to `AGENTS.md`.

#### Examples

```bash
graphify-rs droid install
graphify-rs droid uninstall
```

---

### `graphify-rs trae install` / `uninstall`

Project-level Trae integration. Adds graph instructions to `AGENTS.md`.

#### Examples

```bash
graphify-rs trae install
graphify-rs trae uninstall
```

---

### `graphify-rs trae-cn install` / `uninstall`

Project-level Trae CN integration. Adds graph instructions to `AGENTS.md`.

#### Examples

```bash
graphify-rs trae-cn install
graphify-rs trae-cn uninstall
```

---

### `graphify-rs install`

Install the graphify skill globally for an AI coding assistant platform. Writes the `SKILL.md` file to the platform's skill directory and registers it in the platform's config.

#### Parameters

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--platform <NAME>` | `String` | `"claude"` | Platform to install for. Valid values: `claude`, `codex`, `opencode`, `claw`, `droid`, `trae`, `trae-cn`, `codebuddy`, `windows`. |

#### Skill File Locations

| Platform | Skill Path |
|----------|-----------|
| `claude` | `~/.claude/skills/graphify/SKILL.md` |
| `codex` | `~/.codex/skills/graphify/SKILL.md` |
| `opencode` | `~/.config/opencode/skills/graphify/SKILL.md` |
| `claw` | `~/.claw/skills/graphify/SKILL.md` |
| `droid` | `~/.factory/skills/graphify/SKILL.md` |
| `trae` | `~/.trae/skills/graphify/SKILL.md` |
| `trae-cn` | `~/.trae-cn/skills/graphify/SKILL.md` |
| `codebuddy` | `~/.codebuddy/skills/graphify/SKILL.md` |
| `windows` | `~/.claude/skills/graphify/SKILL.md` |

#### Examples

```bash
# Install for Claude (default)
graphify-rs install

# Install for Codex
graphify-rs install --platform codex

# Install for OpenCode
graphify-rs install --platform opencode
```

---

### `graphify-rs init`

Initialize a `graphify.toml` configuration file in the current directory with commented-out defaults. Fails if the file already exists.

#### Examples

```bash
graphify-rs init
```

Generated file:

```toml
# graphify-rs configuration
# These values serve as defaults and can be overridden by CLI flags.

# Output directory for graph files
# output = ".graphify"

# Disable LLM-based semantic extraction
# no_llm = false

# Only process code files (skip docs/papers)
# code_only = false

# Export formats (comma-separated). Available: json,html,graphml,cypher,svg,wiki,obsidian,report
# Leave empty or omit for all formats.
# formats = ["json", "html", "report"]
```

---

### `graphify-rs completions`

Generate shell completion scripts.

#### Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `<SHELL>` (positional) | `Shell` | *required* | Shell to generate completions for. Values: `bash`, `zsh`, `fish`, `elvish`, `powershell`. |

#### Examples

```bash
# Bash
graphify-rs completions bash > ~/.bash_completion.d/graphify-rs

# Zsh
graphify-rs completions zsh > ~/.zfunc/_graphify-rs

# Fish
graphify-rs completions fish > ~/.config/fish/completions/graphify-rs.fish

# PowerShell
graphify-rs completions powershell > graphify-rs.ps1
```

---

### `graphify-rs benchmark`

Run a token-efficiency benchmark against a graph file.

#### Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `<GRAPH_PATH>` (positional) | `String` | `".graphify/graph.json"` | Path to the graph JSON file. |

#### Examples

```bash
# Benchmark default graph
graphify-rs benchmark

# Benchmark a specific graph
graphify-rs benchmark /path/to/graph.json
```

---

### `graphify-rs save-result`

Save a query result to the memory directory for future reference.

#### Parameters

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--question <TEXT>` | `String` | *required* | The question that was asked. |
| `--answer <TEXT>` | `String` | *required* | The answer that was generated. |
| `--type <TYPE>` | `String` | `"query"` | Result type identifier. |
| `--nodes <ID>...` | `Vec<String>` | `[]` | Related node IDs (can be specified multiple times). |
| `--memory-dir <DIR>` | `String` | `".graphify/memory"` | Directory to save the result in. |

#### Examples

```bash
# Save a query result
graphify-rs save-result \
  --question "How does auth work?" \
  --answer "Auth uses JWT tokens via the auth module..." \
  --type query \
  --nodes auth_module --nodes jwt_handler

# Save to custom memory directory
graphify-rs save-result \
  --question "DB schema" \
  --answer "Uses PostgreSQL with 12 tables..." \
  --memory-dir my-graph/memory
```

---

## Configuration (`graphify.toml`)

Create a `graphify.toml` file in your project root (or run `graphify-rs init`) to set project-level defaults.

### Fields

| Field | Type | Default | CLI Override | Description |
|-------|------|---------|-------------|-------------|
| `output` | `String` | `".graphify"` | `--output` | Output directory for graph files. |
| `no_llm` | `bool` | `false` | `--no-llm` | Disable LLM-based semantic extraction. |
| `code_only` | `bool` | `false` | `--code-only` | Only process code files (skip docs/papers). |
| `formats` | `String[]` | `[]` (all formats) | `--format` | Export formats to generate. |
| `embed` | `bool` | `false` | `--embed` | Build the local Model2Vec semantic index. |
| `embedding_model` | `String` | `minishlab/potion-code-16M` | `--embedding-model` | Model2Vec model ID or local model directory. |

### Precedence Rules

1. **CLI flags** always take the highest priority.
2. **`graphify.toml`** values are used as defaults when CLI flags are not set.
3. **Built-in defaults** are used when neither CLI nor config specifies a value.

Specific merging rules:
- `output`: CLI value is used if it differs from the built-in default (`".graphify"`); otherwise falls back to config.
- `no_llm`: `true` if **either** CLI flag or config is `true` (OR logic).
- `code_only`: `true` if **either** CLI flag or config is `true` (OR logic).
- `formats`: CLI value is used if non-empty; otherwise falls back to config. Empty means all formats.
- `embed`: `true` if **either** CLI flag or config is `true` (OR logic).
- `embedding_model`: CLI value wins when explicitly set; otherwise config or the built-in default is used.

### Example

```toml
# Always output to a custom directory
output = "knowledge-graph"

# Skip Claude API calls by default
no_llm = true

# Only generate JSON and HTML
formats = ["json", "html"]

# Keep local semantic search vectors enabled for agent installs
embed = true
embedding_model = "minishlab/potion-code-16M"
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| `ANTHROPIC_API_KEY` | Required for semantic extraction (pass 2). Without it, only AST extraction runs for doc/paper files. |
| `RUST_LOG` | Log level filter (default: `warn`). Overridden by `-v` (`debug`) or `-q` (`error`). |

---

## Agent Integration

Complete guide for setting up `graphify-rs` as an AI coding agent skill.

### Platform Setup

#### Claude Code

```bash
# 1. Install project-level integration
graphify-rs claude install

# 2. Build the graph with local semantic search
graphify-rs build --no-llm --embed

# 3. (Optional) Install global skill for /graphify slash command
graphify-rs install --platform claude
```

What `claude install` creates:
- `./CLAUDE.md` — appends a `## graphify` section with agent rules
- `.claude/settings.json` — adds a `PreToolUse` hook on `Glob|Grep` that reminds the agent to check the graph first

#### Codex

```bash
# 1. Install project-level integration
graphify-rs codex install

# 2. Build the graph with local semantic search
graphify-rs build --no-llm --embed

# 3. (Optional) Install global skill
graphify-rs install --platform codex
```

What `codex install` creates:
- `./AGENTS.md` — appends a `## graphify` section with agent rules
- `.codex/hooks.json` — adds a `PreToolUse` hook on `Bash` tool calls

#### OpenCode

```bash
# 1. Install project-level integration
graphify-rs opencode install

# 2. Build the graph with local semantic search
graphify-rs build --no-llm --embed

# 3. (Optional) Install global skill
graphify-rs install --platform opencode
```

What `opencode install` creates:
- `./AGENTS.md` — appends a `## graphify` section with agent rules
- `.opencode/plugins/graphify.js` — PreToolUse plugin
- `opencode.json` — registers the plugin

#### CodeBuddy

```bash
# 1. Install project-level integration
graphify-rs codebuddy install

# 2. Build the graph with local semantic search
graphify-rs build --no-llm --embed

# 3. (Optional) Install global skill
graphify-rs install --platform codebuddy
```

What `codebuddy install` creates:
- `./AGENTS.md` — appends a `## graphify` section with agent rules
- `.codebuddy/settings.json` — adds a `PreToolUse` hook on `Glob|Grep` tool calls

#### Claw / Droid / Trae / Trae CN

```bash
graphify-rs claw install       # or droid, trae, trae-cn
graphify-rs build --no-llm --embed
```

These platforms use a generic integration that only writes the `## graphify` section to `./AGENTS.md`.

### How Agents Use the Graph

Once installed, the agent follows these rules (injected into `CLAUDE.md` or `AGENTS.md`):

1. **Before answering architecture or codebase questions** — prefer `graphifyq query "<question>"`; it uses the local Model2Vec semantic index by default and auto-refreshes stale graphs every 300s.
2. **For broad orientation** — read `.graphify/GRAPH_REPORT.md` for god nodes and community structure.
3. **If `.graphify/wiki/index.md` exists** — navigate the wiki instead of reading raw files.
4. **For strict AST-only/offline startup** — pass `--no-embed` to `graphifyq ensure/query`.
5. **After modifying code files** — run `graphifyq ensure` to keep graph.json and semantic-index.json current; force `graphify-rs build --path . --output .graphify --no-llm --update --embed` only when an immediate rebuild is required.

The `PreToolUse` hook automatically fires when the agent uses `Glob` or `Grep` tools (Claude/CodeBuddy) or `Bash` (Codex), injecting a reminder to check the graph first.

### MCP Server Integration

For deeper integration, run the MCP server so the agent can call graph tools directly.

#### Claude Desktop Configuration

Add to your Claude Desktop MCP config (`claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "graphify": {
      "command": "graphify-rs",
      "args": ["serve", "--graph", ".graphify/graph.json"]
    }
  }
}
```

#### Claude Code MCP Configuration

Add to `.claude/settings.json`:

```json
{
  "mcpServers": {
    "graphify": {
      "command": "graphify-rs",
      "args": ["serve", "--graph", ".graphify/graph.json"]
    }
  }
}
```

The agent can then call tools like `query_graph`, `get_node`, `get_neighbors`, `god_nodes`, `graph_stats`, `get_community`, and `shortest_path` directly through the MCP protocol.

### Keeping the Graph Current After Code Changes

```bash
# Fast incremental rebuild (AST-only, ~2-5 seconds)
graphify-rs build --no-llm --update --embed

# Or use watch mode for automatic rebuilds
graphify-rs watch

# Or install git hooks for rebuild on commit
graphify-rs hook install
```

### Version Staleness

`graphify-rs` checks skill file versions on every invocation. If the installed skill was written by a different version of `graphify-rs`, a warning is printed:

```
warning: skill is from graphify-rs 0.2.0, package is 0.3.0. Run 'graphify-rs install' to update.
```

Run `graphify-rs install` to update the skill file.
