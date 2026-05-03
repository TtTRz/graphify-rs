# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.5] - 2026-04-25

### Fixed
- **OpenCode integration** — some OpenCode issues fixed:
  - `install.rs`: fixed `opencode.json` incorrect config node name (was `pluigns`, must be `plugin`)

## [0.4.4] - 2026-04-14

### Fixed
- **Clippy warnings resolved** — 6 warnings across 4 files fixed:
  - `treesitter.rs`: Merged identical `get`/`set` prefix stripping branches
  - `lib.rs`: Collapsed nested `if` into let-chain for Rust 2024
  - `embedding.rs`: Replaced loop variable indexing with slice iterator
  - `temporal.rs`: Removed needless borrow in `date_to_age()` call

## [0.4.3] - 2026-04-14

### Fixed
- **Dart extraction — Critical fixes for 70-80% missing edges**
  - `treesitter.rs`: Added missing `function_declaration` and `method_definition` types (fixes 40-60% of functions)
  - `treesitter.rs`: Added `part_directive` and `part_of_directive` for Dart file-splitting (fixes cross-file relationships)
  - `lib.rs`: Rewrote `resolve_dart_import()` to handle aliased imports (`as`), deferred imports, relative paths (`../`), and part directives (fixes 30-50% of imports)
  - `treesitter.rs`: Added function name normalization for Dart getters/setters (strips `get`/`set` prefixes)
- **Ruby import handler** — `require`/`require_relative` now produce clean module names instead of raw AST text; non-require `call` nodes no longer intercepted as imports
- **Python star import** — `from x import *` now correctly detected even when prior import statements exist (was counting total edges instead of per-statement delta)
- **Java static import** — `import static java.util.Arrays.asList` no longer mis-parsed due to nested `unwrap_or_else` confusion
- **Dart function signatures** — `function_signature`/`method_signature` nodes without `name` field now fall back to first `identifier` child
- **JS async functions** — added `async_function_declaration` to tree-sitter config (was only covered by regex fallback)
- **Ruby no-parens call inference** — call-graph heuristic now detects Ruby-style `bark` calls without parentheses via word-boundary matching
- **NodeType classification for all languages** — `classify_class_kind()` now returns correct types for C/C++/C#/Java/Dart/Ruby (struct→Struct, enum→Enum, interface→Interface, namespace→Namespace, module→Module); was previously falling through to Class for all non-Rust types
- **Dart import handler** — dedicated `extract_dart_import()` properly strips `deferred as`/`show`/`hide` suffixes from import paths

## [0.4.2] - 2026-04-14

### Fixed
- **Skill file sync** — installed skill at `~/.claude/skills/` now matches repo (was stuck at 7 tools, updated to 15)
- **`--max-viz-nodes` flag** added to skill.md available flags

## [0.4.1] - 2026-04-13

### Added
- **SECURITY.md** — responsible vulnerability disclosure process
- **ARCHITECTURE.md** — detailed 14-crate design, algorithm table, MCP tool reference, dependency graph
- **Examples** — `examples/build_and_query.rs` (full pipeline) + `examples/custom_graph.rs` (programmatic API)
- **Criterion benchmarks** — `benches/graphify_bench.rs` with 6 benchmarks (clustering, PageRank, cycles, export, extraction)
- **CI/downloads/docs.rs badges** — README now shows build status, crate downloads, and docs link

### Changed
- **README redesigned** — centered header, streamlined sections, Quick Start with `--no-llm` first, performance table, slim architecture overview with links to detailed docs
- **CONTRIBUTING.md expanded** — architecture overview, test expectations, PR checklist, release process
- **CLI docs with TOC** — CLI.md now has a table of contents for navigation

## [0.4.0] - 2026-04-13

### Added
- **8 new MCP tools** — `find_all_paths`, `weighted_path`, `community_bridges`, `graph_diff`, `pagerank`, `detect_cycles`, `smart_summary`, `find_similar`. Total MCP tools: 7 → 15
- **PageRank algorithm** — power iteration with configurable damping (0.85) and convergence detection; identifies structurally critical nodes beyond simple degree ranking
- **Dependency cycle detection** — Tarjan's SCC algorithm finds circular dependencies (imports/uses/calls); severity scored by cycle length
- **Smart graph summarization** — three abstraction levels for LLM token budgets: `Detailed` (full graph), `Community` (one representative per community + cross-community edges), `Architecture` (directory-level super-nodes with aggregated dependencies)
- **Graph embedding + similarity** — Node2Vec random walks + Skip-gram SGD learns 64-dim node embeddings; cosine similarity finds structurally similar node pairs (redundancy/refactoring candidates)
- **Temporal risk analysis** — git blame integration correlates change frequency × connectivity to identify high-risk nodes (`temporal.rs`)
- **Incremental community detection** — `cluster_incremental()` re-clusters only affected communities when files change; falls back to full Leiden when >50% communities affected
- **Weighted graph analysis** — `confidence_to_weight()` maps EXTRACTED→1.0, INFERRED→0.7, AMBIGUOUS→0.3; `BridgeNode` model for bridge analysis
- **Cross-file import resolution for all 21 languages** — was only Python/JS/Rust/Go; now includes Java, C#, C/C++, Kotlin, PHP, Dart, Scala, Swift (language-specific resolvers for dot imports, backslash imports, C includes, Dart packages)
- **378 tests** covering all 21 supported languages, organized in `tests/` directory per Rust conventions
- **`--max-viz-nodes` flag** — configurable HTML visualization node limit (default 2000), allows larger projects to show more context

### Changed
- **Leiden clustering 10-50x faster** — pre-computed `sigma_c` and `ki_cache` with incremental updates; single-pass neighbor aggregation replaces per-community scans; `merge_small_communities()` uses incremental `node_to_cid`
- **Cross-file import resolution ~100x faster** — `id_to_label` HashMap O(1) lookup replaces O(n) linear scan per import edge; index building consolidated from 6 passes to 2
- **JSON export streaming** — `write_node_link_json()` writes directly to `BufWriter<File>` via `serde_json::Serializer`, eliminating ~500 MB intermediate `Value` + `String` for large graphs
- **Parallel file extraction** — `rayon::par_iter` for concurrent AST extraction and cache lookups; ~6x speedup on 8-core machines

### Fixed
- **God Nodes community column showing "–"** — `cluster()` never wrote back to `node.community`; added `get_node_mut()` and post-clustering community assignment
- **God Nodes duplicate "lib" labels** — multiple crates with `lib.rs` all showed "lib"; added `disambiguate_label()` to prefix with crate name (e.g., `graphify-export::lib`)
- **Semaphore unwrap panic** — `sem.acquire().await.unwrap()` replaced with proper error propagation via `map_err()`

## [0.3.1] - 2026-04-13

### Fixed
- **File name too long (os error 63)** — Obsidian/Wiki export used node labels/IDs as filenames without length limit, causing crashes on macOS (255-byte limit) when analyzing Dart or other languages with long identifiers. Added `truncate_to_bytes()` utility (240-byte cap) to `graphify-core`, applied in `obsidian.rs` and `wiki.rs`

## [0.3.0] - 2026-04-13

### Added
- **Dart language support** — tree-sitter grammar + AST extraction (21 languages total)
- **Skill file** (`skill.md`) — comprehensive AI agent guide with all commands, rebuild rules, and MCP setup
- **Version staleness check** — warns on startup if installed skill is from an older version
- **`.graphify_version` stamp** — written during `graphify-rs install` for staleness detection
- **Small community merging** — communities with < 5 nodes automatically merged into most-connected neighbor
- **Smart community labeling** — picks descriptive function/struct names instead of generic "lib"
- **Graph rebuild instructions** — skill and CLAUDE.md now instruct agents to rebuild after code changes

### Changed
- **tree-sitter upgraded** — core `0.24` → `0.26.8`, grammars to latest (python 0.25, go 0.25, rust 0.24, etc.)
- **Leiden resolution parameter** — lowered from 1.0 to 0.3, reducing over-fragmentation (140 → ~64 communities on same codebase)
- **Command name consistency** — all user-facing strings now use `graphify-rs` instead of `graphify` (git hooks, skill, install messages, hook JSON, OpenCode plugin, report footer, benchmark banner)
- **Claude Code hook format** — aligned with Python original: `hookEventName` + `additionalContext` instead of `prefix`
- **Codex hooks.json format** — aligned with Python original: `PreToolUse` array + `systemMessage`
- **CLAUDE.md rebuild rule** — full command `graphify-rs build --path . --output .graphify --no-llm --update`

### Fixed
- **God Nodes degree=0** — report showed degree 0 for all god nodes due to JSON field name mismatch (`"edges"` → `"degree"`)
- **God Nodes missing community** — `"community"` field was not included in JSON passed to report generator
- **Clippy warnings** — fixed 25 `collapsible_if` + 1 `let_and_return` across 14 files using Rust 2024 let-chains

## [0.2.0] - 2026-04-10

### Added
- **Split HTML export** — `export_html_split()` generates per-community HTML pages with overview navigation
- **Auto-pruning for large graphs** — HTML viz auto-prunes to top-degree + community representative nodes for graphs > 2000 nodes
- **Barnes-Hut physics** — enabled for graphs > 500 nodes, disabled after stabilization
- **Debounced search** — HTML search input debounced 200ms + batch `nodes.update()` to prevent UI lag
- **Shell completions** — `graphify-rs completions bash/zsh/fish` via clap_complete
- **`graphify.toml` config** — project-level configuration file support
- **`--quiet` / `--verbose` flags** — global verbosity control
- **`--jobs` flag** — configurable parallelism for rayon thread pool
- **`--format` flag** — select specific export formats (json, html, svg, graphml, cypher, wiki, obsidian, report)
- **`graphify-rs stats`** — show graph statistics without rebuilding
- **`graphify-rs diff`** — compare two graph snapshots
- **`graphify-rs init`** — create graphify.toml config file
- **Error recovery** — `catch_unwind` for extraction, continues on individual file failures
- **Parallel semantic extraction** — tokio::sync::Semaphore for concurrent Claude API calls
- **Watch incremental rebuild** — only re-extracts changed files via cache invalidation
- **Progress bars** — indicatif progress bars for file extraction
- **Colored output** — colored terminal output via `colored` crate
- **Open source community files** — CONTRIBUTING.md, CODE_OF_CONDUCT.md, SECURITY.md

### Changed
- **Leiden algorithm** — replaced Louvain with Leiden (refinement phase ensures internally connected communities)
- **Rust Edition 2024** — migrated from 2021, using implicit borrowing patterns
- **Multi-platform install** — Claude, Codex, OpenCode, Claw, Droid, Trae, Trae-CN support

### Fixed
- **UTF-8 truncation panic** — `&content[..N]` panics on Chinese/CJK text; fixed with `is_char_boundary()` backward search
- **HTML visualization crash on large graphs** — out-of-memory on > 2000 nodes; fixed with auto-pruning
- **Search performance** — `nodes.update()` called per-node on every keystroke; fixed with debounce + batch update

## [0.1.0] - 2026-04-08

### Added
- Initial Rust rewrite of Python graphify
- 14-crate workspace architecture
- tree-sitter AST extraction for 20 languages
- Claude API semantic extraction (Pass 2)
- Leiden community detection
- 9 export formats: JSON, HTML, SVG, GraphML, Cypher, Wiki, Obsidian, Report
- MCP server with 7 query tools (query_graph, get_node, get_neighbors, get_community, god_nodes, graph_stats, shortest_path)
- SHA256 file-level caching
- Security: URL/path/label validation
- URL ingestion: Twitter, arXiv, PDF, webpage
- File watching with debounce
- Git hook integration (post-commit, post-checkout)
- CLI with 21 subcommands via clap derive

[0.4.5]: https://github.com/TtTRz/graphify-rs/compare/v0.4.4...v0.4.5
[0.4.4]: https://github.com/TtTRz/graphify-rs/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/TtTRz/graphify-rs/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/TtTRz/graphify-rs/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/TtTRz/graphify-rs/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/TtTRz/graphify-rs/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/TtTRz/graphify-rs/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/TtTRz/graphify-rs/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/TtTRz/graphify-rs/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/TtTRz/graphify-rs/releases/tag/v0.1.0
