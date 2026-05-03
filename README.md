<div align="center">

# graphify-rs

**AI-powered knowledge graph builder**

*Transform code, docs, papers, and images into queryable, interactive knowledge graphs.*

[![CI](https://github.com/TtTRz/graphify-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/TtTRz/graphify-rs/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/graphify-rs.svg)](https://crates.io/crates/graphify-rs)
[![Downloads](https://img.shields.io/crates/d/graphify-rs.svg)](https://crates.io/crates/graphify-rs)
[![docs.rs](https://docs.rs/graphify-rs/badge.svg)](https://docs.rs/graphify-rs)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

[CLI Reference](docs/CLI.md) | [Architecture](docs/ARCHITECTURE.md) | [Changelog](CHANGELOG.md)

</div>

---

## Why graphify-rs?

Built around [Andrej Karpathy's /raw folder workflow](https://x.com/karpathy/status/1871129915774632404): drop anything into a folder — papers, tweets, screenshots, code, notes — and get a structured knowledge graph that shows you what you didn't know was connected.

Three things it does that an **LLM alone cannot**:

| | Feature | Why it matters |
|---|---------|---------------|
| 1 | **Persistent graph** | Relationships survive across sessions. Query weeks later without re-reading. |
| 2 | **Honest audit trail** | Every edge tagged `EXTRACTED`, `INFERRED`, or `AMBIGUOUS`. Facts vs. guesses, always clear. |
| 3 | **Cross-document surprise** | Community detection finds connections you'd never think to ask about. |

## Quick Start

```bash
# Install
cargo install graphify-rs

# Build a knowledge graph (free, fast, no API key needed)
graphify-rs build --no-llm

# Explore interactively
open .graphify/graph.html         # macOS
# xdg-open .graphify/graph.html   # Linux

# Query the graph
    graphify-rs query "how does auth work?"

    # Short-lived Codex-friendly query helper
    graphifyq ensure
    graphifyq query "how does auth work?"

# (Optional) Add semantic extraction via Claude API
export ANTHROPIC_API_KEY=sk-...
graphify-rs build
```

## Performance

Rust rewrite of [graphify](https://github.com/safishamsi/graphify) (Python) — fully compatible `graph.json` output.

| | Python | Rust |
|---|--------|------|
| **Speed** | ~204ms | **~24ms** (8.5x faster) |
| **Memory** | ~48MB | **~1MB** (48x less) |
| **AST parsing** | Regex only | 11 native tree-sitter + regex fallback |
| **Community detection** | Louvain | **Leiden** (with refinement) |
| **MCP server** | - | **15 tools** over JSON-RPC 2.0 |
| **Export formats** | 7 | **9** (+ Obsidian, split HTML) |
| **Extraction** | Sequential | **Parallel** (`rayon`, configurable `-j`) |

## How It Works

```
 Source Files              graphify-rs build
 ┌──────────┐    ┌──────────────────────────────────────────────────────┐
 │ .py .rs  │    │                                                      │
 │ .go .ts  │───>│  detect -> extract -> build -> cluster -> analyze -> export
 │ .md .pdf │    │                                                      │
 └──────────┘    └──────────┬───────────────────────────────────────────┘
                            v
                  .graphify/
                  ├── graph.json          queryable graph data
                  ├── graph.html          interactive visualization
                  ├── GRAPH_REPORT.md     analysis report
                  ├── wiki/               per-community wiki pages
                  └── obsidian/           Obsidian vault
```

**Pass 1 — AST extraction** (free, always runs): tree-sitter parses 21 languages into functions, classes, imports, calls. All edges tagged `EXTRACTED` (confidence 1.0).

**Pass 2 — Semantic extraction** (optional, `--no-llm` to skip): Claude API discovers conceptual links, shared assumptions, design rationale. Edges tagged `INFERRED` (confidence 0.4–0.9).

## Graph Algorithms

7 advanced algorithms beyond basic traversal:

| Algorithm | What it does |
|-----------|-------------|
| **Leiden clustering** | Community detection with internal connectivity guarantee |
| **PageRank** | Structural importance (not just degree) — finds true architectural pillars |
| **Tarjan's SCC** | Dependency cycle detection — surfaces circular imports |
| **Dijkstra weighted path** | Shortest path weighted by edge confidence |
| **Node2Vec embedding** | Graph similarity search — finds redundant/refactorable code |
| **Incremental clustering** | Re-clusters only changed communities on rebuild |
| **Smart summarization** | Three-level abstraction (detailed → community → architecture) for LLM token budgets |

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#graph-algorithms) for complexity analysis.

## Supported Languages (22)

| Native tree-sitter | SQL parser | Regex fallback |
|---------------------|------------|----------------|
| Python, JavaScript, TypeScript, Rust, Go, Java, C, C++, Ruby, C#, Dart | PostgreSQL + ClickHouse `.sql` via `sqlparser` | Kotlin, Scala, PHP, Swift, Lua, Zig, PowerShell, Elixir, Obj-C, Julia |

## Agent Integration

```bash
graphify-rs install              # install skill for AI coding agents
graphify-rs serve                # start MCP server (15 tools)
```

Agents auto-check the graph before architecture questions and rebuild after code changes. Works with Claude Code, CodeBuddy, Codex, OpenCode, and more.

15 MCP tools: `query_graph`, `pagerank`, `detect_cycles`, `smart_summary`, `find_similar`, `shortest_path`, and [9 more](docs/ARCHITECTURE.md#mcp-server-tools-15).

## Architecture

14-crate Cargo workspace — see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full design.

| Crate | Role |
|-------|------|
| `graphify-core` | Data models, graph structure, confidence system |
| `graphify-extract` | AST extraction (21 languages), Claude API semantic extraction |
| `graphify-cluster` | Leiden community detection, incremental re-clustering |
| `graphify-analyze` | PageRank, cycles, embeddings, god nodes, temporal risk |
| `graphify-serve` | MCP server (15 tools), smart summarization |
| `graphify-export` | 9 formats: JSON, HTML, SVG, GraphML, Cypher, Wiki, Obsidian, Report |
| + 8 more | Cache, security, ingestion, watch, hooks, benchmark, detect, build |

## Output Formats

| File | Description |
|------|-------------|
| `graph.json` | NetworkX-compatible `node_link_data` JSON |
| `graph.html` | Interactive vis.js visualization (dark theme, auto-pruning) |
| `html/` | Per-community HTML pages with navigation |
| `GRAPH_REPORT.md` | God nodes, surprising connections, suggested questions |
| `graph.svg` / `graph.graphml` | Static visualization / graph editor import |
| `cypher.txt` | Neo4j import script |
| `wiki/` / `obsidian/` | Wiki pages / Obsidian vault with wikilinks |

## CLI at a Glance

```bash
graphify-rs build [--path .] [--no-llm] [--format json,html]   # build graph
graphify-rs query "question" [--dfs] [--budget 2000]            # query
graphify-rs watch --path .                                       # auto-rebuild
    graphify-rs serve                                                 # MCP stdio server
    graphify-rs serve --transport http --registry-path .graphify/.graphifyq-server.json
    graphifyq query "where is auth wired?"                            # reuse local HTTP sidecar
graphify-rs diff old.json new.json                               # compare
graphify-rs stats graph.json                                     # statistics
```

Full reference: **[docs/CLI.md](docs/CLI.md)** (22 subcommands)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, code style, testing, and PR guidelines.

## License

MIT — see [LICENSE](LICENSE).

Rust rewrite of [graphify](https://github.com/safishamsi/graphify) by safishamsi.
