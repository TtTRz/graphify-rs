# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
- **CLAUDE.md rebuild rule** — full command `graphify-rs build --path . --output graphify-out --no-llm --update`

### Fixed
- **God Nodes degree=0** — report showed degree 0 for all god nodes due to JSON field name mismatch (`"edges"` → `"degree"`)
- **God Nodes missing community** — `"community"` field was not included in JSON passed to report generator
- **File name too long (os error 63)** — Obsidian/Wiki export used node labels/IDs as filenames without length limit; added `truncate_to_bytes()` utility (240-byte cap) to `graphify-core`, applied in `obsidian.rs` and `wiki.rs`
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

[0.3.0]: https://github.com/TtTRz/graphify-rs/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/TtTRz/graphify-rs/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/TtTRz/graphify-rs/releases/tag/v0.1.0
