# Architecture

graphify-rs is organized as a 15-crate Cargo workspace. Each crate has a single responsibility and communicates through shared types defined in `graphify-core`.

## Pipeline

```
Source Files → detect → extract → build → cluster → analyze → export
                 │         │                  │         │         │
                 ▼         ▼                  ▼         ▼         ▼
             .graphify  tree-sitter        Leiden    PageRank   JSON, HTML,
              ignore    + regex AST       + merge   + Tarjan    SVG, Report,
                        + Claude API                + embed    Obsidian, ...
                                                     + Model2Vec query index
```

## Crate Map

| Crate | Purpose | Key Functions |
|-------|---------|---------------|
| `graphify-core` | Data models, graph structure, ID generation, confidence system | `KnowledgeGraph`, `GraphNode`, `GraphEdge` |
| `graphify-detect` | File discovery, classification, `.gitignore`/`.graphifyignore`, sensitive file filtering | `classify_file()`, `is_sensitive()` |
| `graphify-extract` | AST extraction (21 languages via tree-sitter + regex), Claude API semantic extraction | `extract()`, `extract_file()`, `resolve_cross_file_imports()` |
| `graphify-build` | Graph assembly from extraction results, node/edge deduplication | `build_from_extraction()` |
| `graphify-cluster` | Leiden community detection, cohesion scoring, incremental re-clustering | `cluster()`, `cluster_incremental()`, `cohesion_score()` |
| `graphify-analyze` | PageRank, dependency cycles, god nodes, surprising connections, graph embeddings, temporal risk | `pagerank()`, `detect_cycles()`, `god_nodes()` |
| `graphify-embed` | Optional Model2Vec semantic index for natural-language graph search | `build_model2vec_index()`, `score_hybrid()` |
| `graphify-export` | 9 formats: JSON, HTML, split HTML, SVG, GraphML, Cypher, Wiki, Report, Obsidian | `export_json()`, `export_html()` |
| `graphify-cache` | SHA256 content-hash caching for incremental rebuilds | `load_cached_from()`, `save_cached_to()` |
| `graphify-security` | URL validation (SSRF), path traversal protection, label injection defense | `validate_url()`, `sanitize_path()` |
| `graphify-ingest` | URL fetching: arXiv, tweets (oEmbed), PDFs, webpages | `ingest_url()` |
| `graphify-serve` | MCP server with 16 query tools over JSON-RPC 2.0 stdio | `dispatch()`, `smart_summary()` |
| `graphify-watch` | File monitoring with debounce, incremental rebuild | `watch()` |
| `graphify-hooks` | Git hook install/uninstall (post-commit, post-checkout) | `install()`, `uninstall()` |
| `graphify-benchmark` | Token efficiency measurement | `benchmark()` |

## Graph Algorithms

| Algorithm | Module | Purpose | Complexity |
|-----------|--------|---------|------------|
| **Leiden clustering** | `graphify-cluster` | Community detection with refinement guarantee | O(n·deg) per iteration |
| **Incremental Leiden** | `graphify-cluster` | Re-cluster only affected communities on file change | O(delta·deg) |
| **PageRank** | `graphify-analyze` | Identify structurally critical nodes (not just high-degree) | O(20·(n+m)) |
| **Tarjan's SCC** | `graphify-analyze` | Detect circular dependency chains | O(n+m) |
| **Node2Vec embedding** | `graphify-analyze` | Learn node representations for similarity search | O(walks·n·dim) |
| **Semantic index** | `graphify-embed` | Rank graph nodes by natural-language meaning via Model2Vec, Ollama, or Voyage plus lexical/degree/quality boosts | O(n·dim) per query |
| **Temporal risk** | `graphify-analyze` | Correlate git churn with graph connectivity | O(n·git_log) |
| **Dijkstra weighted path** | `graphify-serve` | Shortest path weighted by edge confidence | O((n+m) log n) |
| **Smart summarization** | `graphify-serve` | Three-level abstraction for LLM token budgets | O(n+m) |

## MCP Server Tools (16)

| Tool | Category | Description |
|------|----------|-------------|
| `query_graph` | Search | Search nodes by keywords or semantic index, return subgraph context |
| `semantic_query` | Search | Return ranked semantic node matches from Model2Vec/Ollama/Voyage index |
| `get_node` | Explore | Get detailed info about a specific node |
| `get_neighbors` | Explore | Get a node's neighbors and connecting edges |
| `get_community` | Explore | List all nodes in a community |
| `god_nodes` | Analyze | Find the most-connected hub nodes |
| `pagerank` | Analyze | Compute PageRank importance scores |
| `detect_cycles` | Analyze | Detect dependency cycles (Tarjan SCC) |
| `find_similar` | Analyze | Find structurally similar node pairs via embeddings |
| `community_bridges` | Analyze | Cross-community bridge nodes |
| `graph_stats` | Stats | Overall graph statistics |
| `graph_diff` | Stats | Compare two graph snapshots |
| `shortest_path` | Path | BFS shortest path |
| `find_all_paths` | Path | Enumerate all simple paths (DFS, max 50) |
| `weighted_path` | Path | Dijkstra weighted shortest path |
| `smart_summary` | Summary | Multi-level graph summary (detailed/community/architecture) |

## Confidence System

Every edge carries a confidence tag:

| Tag | Meaning | Score | Source |
|-----|---------|-------|--------|
| `EXTRACTED` | Found directly in source (import, call, citation) | 1.0 | tree-sitter / regex |
| `INFERRED` | Reasonable inference from context | 0.4–0.9 | Optional legacy Claude extraction / cross-file resolution |
| `AMBIGUOUS` | Uncertain — flagged for human review | 0.1–0.3 | Optional legacy Claude extraction |

## Supported Languages (22)

| Native (tree-sitter) | SQL parser | Regex Fallback |
|----------------------|------------|----------------|
| Python, JavaScript, TypeScript, Rust, Go, Java | PostgreSQL + ClickHouse `.sql` via `sqlparser` | Kotlin, Scala, PHP, Swift, Lua |
| C, C++, Ruby, C#, Dart |  | Zig, PowerShell, Elixir, Obj-C, Julia |

## Dependency Graph

```
                        graphify-core
                       /      |      \
                 security   cache   detect
                    |         |       |
                  extract ────┘       │
                  /     \             │
               build   cluster        │
                 \     /              │
                 analyze              │
                    |                 │
                  export              │
                  /    \              │
               serve   watch          │
                 |       |            │
                hooks  benchmark      │
                  \      |           /
                   graphify-rs (bin)
```
