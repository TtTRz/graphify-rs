# graphify-rs

A Rust rewrite of [graphify](https://github.com/safishamsi/graphify) — an AI-powered knowledge graph builder that transforms code, docs, papers, and images into queryable, interactive knowledge graphs.

[中文文档](README_CN.md)

## Compared to the Python Version

**Full feature parity** with the original, plus:

| Area | Python (original) | Rust (this repo) |
|------|-------------------|-------------------|
| Performance | ~204ms, ~48MB RAM | ~24ms, ~1MB RAM (8.5x faster, 48x less memory) |
| AST parsing | Regex only | 10 languages native tree-sitter + regex fallback |
| Semantic extraction | Sequential | Concurrent with configurable parallelism (`-j`) |
| MCP server | Not included | 7 tools over JSON-RPC 2.0 stdio |
| Export formats | 7 | 8 (+ Obsidian vault) |
| CLI | Basic | 21 subcommands, `--quiet`/`--verbose`, shell completions |
| Progress | No feedback | Progress bars for large projects |
| Config | CLI only | `graphify.toml` project-level defaults |
| Watch mode | Full rebuild | Incremental (only changed files re-extracted) |
| Graph diff | Function only | `graphify-rs diff` CLI command with colored output |
| Graph stats | Not available | `graphify-rs stats` standalone command |
| Output | Plain text | Colored terminal output |

Output format is **fully compatible** — `graph.json` uses the same NetworkX `node_link_data` schema, so Python tools can read Rust output and vice versa.

## Quick Start

```bash
cargo install --path .
graphify-rs build
open graphify-out/graph.html
```

## Usage

```bash
# Build
graphify-rs build --path . --output graphify-out
graphify-rs build --format json,html,report    # select export formats
graphify-rs build --code-only                   # skip docs/papers
graphify-rs build --update                      # incremental rebuild
graphify-rs build --no-llm                      # skip Claude API

# Global flags
graphify-rs -q build                            # quiet mode
graphify-rs -v build                            # verbose/debug mode
graphify-rs -j 4 build                          # limit parallel jobs

# Query & analyze
graphify-rs query "how does authentication work?"
graphify-rs diff old/graph.json new/graph.json
graphify-rs stats graphify-out/graph.json

# MCP server (Claude Code integration)
graphify-rs serve --graph graphify-out/graph.json

# Watch & auto-rebuild
graphify-rs watch --path . --output graphify-out

# Ingest URL content
graphify-rs ingest https://arxiv.org/abs/2301.00001

# Git hooks
graphify-rs hook install

# Platform integrations
graphify-rs claude install
graphify-rs codex install

# Shell completions
graphify-rs completions bash > ~/.bash_completion.d/graphify-rs

# Config file
graphify-rs init                                # creates graphify.toml
```

## Configuration

Create `graphify.toml` in your project root (or run `graphify-rs init`):

```toml
output = "graphify-out"
no_llm = false
code_only = false
formats = ["json", "html", "report"]
```

CLI flags always override config file values.

## Architecture

14 crates organized as a Cargo workspace:

| Crate | Purpose |
|-------|---------|
| `graphify-core` | Data models, graph operations, ID generation, confidence system |
| `graphify-detect` | File discovery, classification, .graphifyignore, sensitive file filtering |
| `graphify-extract` | AST extraction (tree-sitter + regex), Claude API semantic extraction |
| `graphify-build` | Graph assembly, deduplication |
| `graphify-cluster` | Community detection (Louvain), cohesion scoring |
| `graphify-analyze` | God nodes, surprising connections, suggested questions, graph diff |
| `graphify-export` | JSON, HTML, SVG, GraphML, Cypher, Wiki, Report, Obsidian |
| `graphify-cache` | SHA256 content-hash caching with atomic writes |
| `graphify-security` | URL/path/label validation, SSRF prevention |
| `graphify-ingest` | URL fetching (arXiv, tweets, PDFs, webpages) |
| `graphify-serve` | MCP server (7 tools), BFS/DFS traversal, scoring |
| `graphify-watch` | File monitoring with debounce, incremental rebuild |
| `graphify-hooks` | Git hook install/uninstall/status |
| `graphify-benchmark` | Token efficiency metrics |

## Output Formats

| File | Description |
|------|-------------|
| `graph.json` | NetworkX-compatible node_link_data JSON |
| `graph.html` | Interactive vis.js visualization (dark theme) |
| `GRAPH_REPORT.md` | Analysis report: communities, god nodes, surprises |
| `graph.svg` | Static graph visualization |
| `graph.graphml` | For graph editors (yEd, Gephi) |
| `cypher.txt` | Neo4j import script |
| `wiki/` | Wiki-style pages per community |
| `obsidian/` | Obsidian vault with wikilinks |

## MCP Server Tools

When running `graphify-rs serve`, 7 tools are available over JSON-RPC 2.0 (stdio):

| Tool | Description |
|------|-------------|
| `query_graph` | Search nodes by keywords, return subgraph context |
| `get_node` | Get detailed info about a specific node |
| `get_neighbors` | Get a node's neighbors and connecting edges |
| `get_community` | List all nodes in a community |
| `god_nodes` | Find the most-connected hub nodes |
| `graph_stats` | Overall graph statistics |
| `shortest_path` | Find shortest path between two nodes |

## Supported Languages

| Native (tree-sitter) | Regex Fallback |
|----------------------|----------------|
| Python, JavaScript, TypeScript, Rust, Go | PHP, Swift, Kotlin, Scala, Dart |
| Java, C, C++, Ruby, C# | Lua, Haskell, Elixir, Shell/Bash, R |

## License

MIT — see [LICENSE](LICENSE).

This project is a Rust rewrite of [graphify](https://github.com/safishamsi/graphify) by safishamsi.
