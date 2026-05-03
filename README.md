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
# Install (macOS/Linux; this fork ships pure-Rust Model2Vec support)
cargo install --git https://github.com/xenking/graphify-rs --locked

# Build a knowledge graph + compact LLM context pack with local semantic search
# (free, no API key needed; Model2Vec is the default embedding backend)
graphify-rs build --no-llm --embed

# Explore interactively
open .graphify/graph.html         # macOS
# xdg-open .graphify/graph.html   # Linux

# Query the graph
graphify-rs query "how does auth work?"

# Short-lived Codex-friendly query helper.
# Model2Vec semantic search is on by default; graphifyq auto-refreshes stale graphs every 300s.
# Use --no-embed for fast/offline AST-only mode or --no-auto-refresh for read-only checks.
graphifyq ensure
graphifyq query "how does auth work?"

# Optional embedding backends
ollama pull embeddinggemma
graphify-rs build --no-llm --embed --embedding-provider ollama --embedding-model embeddinggemma
VOYAGE_API_KEY=... graphify-rs build --no-llm --embed --embedding-provider voyage --embedding-model voyage-code-3
```

## Performance

Rust rewrite of [graphify](https://github.com/safishamsi/graphify) (Python) — fully compatible `graph.json` output.

| | Python | Rust |
|---|--------|------|
| **Speed** | ~204ms | **~24ms** (8.5x faster) |
| **Memory** | ~48MB | **~1MB** (48x less) |
| **AST parsing** | Regex only | 11 native tree-sitter + regex fallback |
| **Community detection** | Louvain | **Leiden** (with refinement) |
| **MCP server** | - | **16 tools** over JSON-RPC 2.0 |
| **Semantic query** | - | **Local Model2Vec index** (`--embed`; default for `graphifyq`), plus Ollama/Voyage backends |
| **Export formats** | 7 | **10** (+ Obsidian, split HTML, LLM context pack) |
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
                  ├── semantic-index.json local/remote semantic search index (default for graphifyq)
                  ├── LLM_CONTEXT.md      compact ranked context pack for agents
                  ├── graph.html          interactive visualization
                  ├── GRAPH_REPORT.md     analysis report
                  ├── wiki/               per-community wiki pages
                  └── obsidian/           Obsidian vault
```

**Pass 1 — AST extraction** (free, always runs): tree-sitter parses 21 languages into functions, classes, imports, calls. All edges tagged `EXTRACTED` (confidence 1.0).

**Pass 1b — Local document context** (free, no API key): Markdown/RST/text docs are indexed into concept nodes so README/PRODUCT/planning docs can anchor LLM context even with `--no-llm`.

**Optional legacy Anthropic extraction** (`--anthropic-semantic`): Claude API concept extraction remains available only by explicit opt-in and requires `ANTHROPIC_API_KEY`. Default builds do not ask for Anthropic keys.

**Semantic query index** (`--embed`, default for `graphifyq ensure/query`): embeddings are stored in `.graphify/semantic-index.json` so `query_graph` / `semantic_query` can rank graph nodes by natural-language meaning before returning relationship-aware graph context. Default backend is local Model2Vec. `--embedding-provider ollama` uses Ollama `/api/embed`; `--embedding-provider voyage` uses Voyage embeddings with `VOYAGE_API_KEY`. `graphifyq` also keeps per-repo graphs fresh with a 300s TTL by running `graphify-rs build --path . --output .graphify --no-llm --update --embed` when stale, then restarting its local HTTP sidecar so queries see the new graph. Use `graphifyq ensure --no-embed` or `graphifyq query --no-embed ...` when you explicitly need AST-only/offline startup; use `--no-auto-refresh` for read-only checks.

**LLM context pack**: `.graphify/LLM_CONTEXT.md` is a compact, ranked first-read artifact for agents. It boosts project docs and production entrypoints while downranking generated/minified/build/test/dependency nodes.


## Ignore Rules

graphify-rs respects repository ignore rules by default: root `.gitignore`, `.git/info/exclude`, and `.graphifyignore`. Use `.graphifyignore` for graph-specific rules; `!path` entries can re-include gitignored files when you explicitly want them in the graph.

You usually do not need to ignore generated code manually. graphify auto-detects codegen signatures (`do not edit`, `code generated`, `*_gen.go`, `*.pb.go`, generated API types), minified bundles, build outputs, vendored deps, and tests, then downranks them in reports, semantic search, and `LLM_CONTEXT.md` instead of deleting provenance from the graph.

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
graphify-rs serve                # start MCP server (16 tools)
```

Agents auto-check the graph before architecture questions and rebuild after code changes. Works with Claude Code, CodeBuddy, Codex, OpenCode, and more.

16 MCP tools: `query_graph`, `semantic_query`, `pagerank`, `detect_cycles`, `smart_summary`, `find_similar`, `shortest_path`, and [9 more](docs/ARCHITECTURE.md#mcp-server-tools-16).

## Architecture

15-crate Cargo workspace — see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full design.

| Crate | Role |
|-------|------|
| `graphify-core` | Data models, graph structure, confidence system |
| `graphify-extract` | AST extraction (21 languages), Claude API semantic extraction |
| `graphify-cluster` | Leiden community detection, incremental re-clustering |
| `graphify-analyze` | PageRank, cycles, embeddings, god nodes, temporal risk |
| `graphify-embed` | Model2Vec semantic index build/query |
| `graphify-serve` | MCP server (16 tools), smart summarization |
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
graphify-rs build [--path .] [--no-llm] [--embed] [--format json,html] # build graph; --embed adds local semantic search
graphify-rs query "question" [--dfs] [--budget 2000]            # query
graphify-rs watch --path .                                       # auto-rebuild
    graphify-rs serve                                                 # MCP stdio server
    graphify-rs serve --transport http --registry-path .graphify/.graphifyq-server.json
    graphifyq query "where is auth wired?"                            # semantic by default; auto-refresh stale graph, reuse local HTTP sidecar
graphify-rs diff old.json new.json                               # compare
graphify-rs stats graph.json                                     # statistics
```

Full reference: **[docs/CLI.md](docs/CLI.md)** (22 subcommands)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, code style, testing, and PR guidelines.

## License

MIT — see [LICENSE](LICENSE).

Rust rewrite of [graphify](https://github.com/safishamsi/graphify) by safishamsi.
