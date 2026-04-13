# graphify-rs

[![Crates.io](https://img.shields.io/crates/v/graphify-rs.svg)](https://crates.io/crates/graphify-rs)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

AI-powered knowledge graph builder — transform code, docs, papers, and images into queryable, interactive knowledge graphs.

[中文文档](README_CN.md) | [CLI Reference](docs/CLI.md) | [Changelog](CHANGELOG.md)

## What is graphify-rs?

**graphify-rs** is built around [Andrej Karpathy's /raw folder workflow](https://x.com/karpathy/status/1871129915774632404): drop anything into a folder — papers, tweets, screenshots, code, notes — and get a structured knowledge graph that shows you what you didn't know was connected.

It is a Rust rewrite of [graphify](https://github.com/safishamsi/graphify) (Python), with full feature parity and significant performance improvements.

### Three things it does that an LLM alone cannot

1. **Persistent graph** — relationships are stored in `graph.json` and survive across sessions. Ask questions weeks later without re-reading everything.
2. **Honest audit trail** — every edge is tagged `EXTRACTED`, `INFERRED`, or `AMBIGUOUS`. You always know what was found in source vs. what was inferred.
3. **Cross-document surprise** — community detection finds connections between concepts in different files that you would never think to ask about directly.

### Use cases

- **New to a codebase** — understand architecture before touching anything
- **Research corpus** — papers + tweets + notes → one navigable graph with citation + concept edges
- **Personal /raw folder** — drop everything in, let it grow, query it anytime
- **Agentic workflows** — AI agents query the graph via MCP server for grounded, structured context

## Compared to the Python Version

| Area | Python (original) | Rust (this repo) |
|------|-------------------|-------------------|
| Performance | ~204ms, ~48MB RAM | ~24ms, ~1MB RAM (8.5x faster, 48x less memory) |
| AST parsing | Regex only | 11 languages native tree-sitter + regex fallback |
| Semantic extraction | Sequential | Concurrent with configurable parallelism (`-j`) |
| Community detection | Louvain (graspologic) | Leiden (hand-written, with refinement phase) |
| MCP server | Not included | 11 tools over JSON-RPC 2.0 stdio |
| Export formats | 7 | 9 (+ Obsidian vault, split HTML per community) |
| CLI | Basic | 21 subcommands, `--quiet`/`--verbose`, shell completions |
| Watch mode | Full rebuild | Incremental (only changed files re-extracted) |

Output format is **fully compatible** — `graph.json` uses the same NetworkX `node_link_data` schema.

## Installation

### From crates.io

```bash
cargo install graphify-rs
```

### From source

```bash
git clone https://github.com/TtTRz/graphify-rs.git
cd graphify-rs
cargo install --path .
```

## Quick Start

```bash
graphify-rs build                    # build knowledge graph from current directory
open graphify-out/graph.html         # explore in browser
graphify-rs query "how does auth work?"  # query the graph
```

For the full CLI reference, see **[docs/CLI.md](docs/CLI.md)**.

## How It Works

### Pipeline Overview

```
 Source Files          graphify-rs build
 ┌──────────┐    ┌─────────────────────────────────────────────────────────┐
 │ .py .rs  │    │                                                         │
 │ .go .ts  │───▶│  detect → extract → build → cluster → analyze → export │
 │ .md .pdf │    │                                                         │
 │ .png     │    └──────────┬──────────────────────────────────────────────┘
 └──────────┘               │
                            ▼
                  graphify-out/
                  ├── graph.json        (queryable graph data)
                  ├── graph.html        (interactive visualization)
                  ├── GRAPH_REPORT.md   (analysis report)
                  ├── wiki/             (per-community wiki pages)
                  └── obsidian/         (Obsidian vault)
```

### Two-Pass Extraction

**Pass 1 — Deterministic AST extraction** (free, fast, always runs):

Uses [tree-sitter](https://tree-sitter.github.io/) to parse source code into ASTs, then extracts functions, classes, imports, and call relationships. Supports 21 languages with 11 native tree-sitter grammars and regex fallback for the rest. Every edge from this pass is tagged `EXTRACTED` with confidence 1.0.

**Pass 2 — Semantic extraction via Claude API** (optional, `--no-llm` to skip):

Sends document/paper/image content to the Claude API to discover higher-level relationships that syntax alone cannot reveal — conceptual links, shared assumptions, design rationale. Edges from this pass are tagged `INFERRED` with confidence scores from 0.4 to 0.9.

### Confidence System

Every edge in the graph carries a confidence tag:

| Tag | Meaning | Score |
|-----|---------|-------|
| `EXTRACTED` | Found directly in source (import, call, citation) | 1.0 |
| `INFERRED` | Reasonable inference from context | 0.4–0.9 |
| `AMBIGUOUS` | Uncertain — flagged for human review | 0.1–0.3 |

This ensures you always know which relationships are facts vs. guesses.

### Leiden Community Detection

After building the graph, graphify-rs runs the [Leiden algorithm](https://www.nature.com/articles/s41598-019-41695-z) to partition nodes into communities:

1. **Louvain phase** — greedy modularity optimization, moving nodes to neighboring communities for maximum modularity gain
2. **Refinement phase** — BFS within each community to ensure internal connectivity; disconnected sub-communities are split
3. **Small community merging** — communities with < 5 nodes are merged into their most-connected neighbor

Each community receives a cohesion score (ratio of actual intra-community edges to maximum possible), and the report surfaces "god nodes" (highest-degree hubs) and "surprising connections" (edges that bridge different communities).

## Architecture

14 crates organized as a Cargo workspace:

| Crate | Purpose |
|-------|---------|
| `graphify-core` | Data models (`GraphNode`, `GraphEdge`, `KnowledgeGraph`), ID generation, confidence system |
| `graphify-detect` | File discovery, classification (code/doc/paper/image), `.graphifyignore`, sensitive file filtering |
| `graphify-extract` | AST extraction (tree-sitter, 21 languages), Claude API semantic extraction, deduplication |
| `graphify-build` | Graph assembly from extraction results, node/edge deduplication |
| `graphify-cluster` | Leiden community detection, cohesion scoring, community splitting/merging |
| `graphify-analyze` | God nodes, surprising connections, suggested questions, graph diff |
| `graphify-export` | 9 formats: JSON, HTML, split HTML, SVG, GraphML, Cypher, Wiki, Report, Obsidian |
| `graphify-cache` | SHA256 content-hash caching for incremental rebuilds |
| `graphify-security` | URL validation (SSRF prevention), path traversal protection, label injection defense |
| `graphify-ingest` | URL fetching: arXiv abstracts, tweets (oEmbed), PDFs, generic webpages |
| `graphify-serve` | MCP server with 11 query tools over JSON-RPC 2.0 stdio |
| `graphify-watch` | File monitoring with debounce, incremental rebuild on code changes |
| `graphify-hooks` | Git hook install/uninstall/status (post-commit, post-checkout) |
| `graphify-benchmark` | Token efficiency measurement (graph tokens vs. raw corpus tokens) |

## Output Formats

| File | Description |
|------|-------------|
| `graph.json` | NetworkX-compatible `node_link_data` JSON |
| `graph.html` | Interactive vis.js visualization (dark theme, auto-pruning for large graphs) |
| `html/` | Per-community HTML pages with overview navigation |
| `GRAPH_REPORT.md` | Analysis report: communities, god nodes, surprises, suggested questions |
| `graph.svg` | Static circular-layout graph visualization |
| `graph.graphml` | For graph editors (yEd, Gephi) |
| `cypher.txt` | Neo4j Cypher import script |
| `wiki/` | Wiki-style markdown pages per community |
| `obsidian/` | Obsidian vault with wikilinks and frontmatter |

## CLI Reference

See **[docs/CLI.md](docs/CLI.md)** for the complete command reference with all flags, defaults, and examples.

Quick overview:

```bash
graphify-rs build [--path .] [--no-llm] [--format json,html]  # build graph
graphify-rs query "question" [--dfs] [--budget 2000]           # query graph
graphify-rs watch --path .                                      # auto-rebuild
graphify-rs serve                                                # MCP server
graphify-rs diff old.json new.json                              # compare graphs
graphify-rs stats graph.json                                    # show statistics
```

## Agent Integration

graphify-rs integrates with AI coding agents (Claude Code, Codex, OpenCode, etc.) via skill installation and MCP server.

```bash
graphify-rs install                # install skill globally
graphify-rs claude install         # project-level: CLAUDE.md + PreToolUse hook
graphify-rs serve                  # start MCP server for agent queries
```

Once installed, agents automatically check the knowledge graph before answering architecture questions and rebuild it after code changes.

For full agent setup instructions, see the [Agent Integration](docs/CLI.md#agent-integration) section of the CLI reference.

### MCP Server Tools

| Tool | Description |
|------|-------------|
| `query_graph` | Search nodes by keywords, return subgraph context |
| `get_node` | Get detailed info about a specific node |
| `get_neighbors` | Get a node's neighbors and connecting edges |
| `get_community` | List all nodes in a community |
| `god_nodes` | Find the most-connected hub nodes |
| `graph_stats` | Overall graph statistics |
| `shortest_path` | Find shortest path between two nodes |

## Supported Languages (21)

| Native (tree-sitter) | Regex Fallback |
|----------------------|----------------|
| Python, JavaScript, TypeScript, Rust, Go, Java | Kotlin, Scala, PHP, Swift, Lua |
| C, C++, Ruby, C#, Dart | Zig, PowerShell, Elixir, Obj-C, Julia |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, code style, and PR guidelines.

## License

MIT — see [LICENSE](LICENSE).

This project is a Rust rewrite of [graphify](https://github.com/safishamsi/graphify) by safishamsi.
