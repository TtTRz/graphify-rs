---
name: graphify
description: any input (code, docs, papers, images) → knowledge graph → clustered communities → HTML + JSON + audit report
trigger: /graphify
---

# /graphify

Turn any folder of files into a navigable knowledge graph with community detection, an honest audit trail, and multiple outputs: interactive HTML, GraphRAG-ready JSON, and a plain-language GRAPH_REPORT.md.

## Usage

```
/graphify                                             # full pipeline on current directory
/graphify <path>                                      # full pipeline on specific path
/graphify <path> --code-only                          # code files only, no LLM needed
/graphify <path> --no-llm                             # skip semantic extraction (AST only)
/graphify <path> --update                             # incremental - re-extract only new/changed files
/graphify <path> --format json,html,report            # select specific export formats
/graphify <path> --format graphml                     # export graph.graphml (Gephi, yEd)
/graphify <path> --format cypher                      # generate .graphify/cypher.txt for Neo4j
/graphify <path> --format svg                         # export graph.svg
/graphify <path> --format wiki                        # build agent-crawlable wiki
/graphify <path> --format obsidian                    # write Obsidian vault
/graphify query "<question>"                          # BFS traversal - broad context
/graphify query "<question>" --dfs                    # DFS - trace a specific path
/graphify query "<question>" --budget 1500            # cap answer at N tokens
/graphify add <url>                                   # fetch URL, save to ./raw, update graph
/graphify watch <path>                                # watch folder, auto-rebuild on code changes
/graphify serve                                       # start MCP stdio server for agent access
graphifyq ensure                                      # build missing graph and start/reuse local HTTP MCP sidecar
graphifyq query "how does auth work?"                 # short-lived HTTP query helper
graphifyq summary architecture                        # architecture summary via MCP smart_summary
```

## What graphify is for

graphify is built around Andrej Karpathy's /raw folder workflow: drop anything into a folder - papers, tweets, screenshots, code, notes - and get a structured knowledge graph that shows you what you didn't know was connected.

Three things it does that an AI coding agent alone cannot:
1. **Persistent graph** - relationships are stored in `.graphify/graph.json` and survive across sessions. Ask questions weeks later without re-reading everything.
2. **Honest audit trail** - every edge is tagged EXTRACTED, INFERRED, or AMBIGUOUS. You know what was found vs invented.
3. **Cross-document surprise** - community detection finds connections between concepts in different files that you would never think to ask about directly.

Use it for:
- A codebase you're new to (understand architecture before touching anything)
- A reading list (papers + tweets + notes → one navigable graph)
- A research corpus (citation graph + concept graph in one)
- Your personal /raw folder (drop everything in, let it grow, query it)

## What You Must Do When Invoked

If no path was given, use `.` (current directory). Do not ask the user for a path.

Follow these steps in order. Do not skip steps.

### Step 1 - Ensure graphify-rs is installed

```bash
if ! command -v graphify-rs >/dev/null 2>&1; then
  echo "graphify-rs not found. Install with: cargo install graphify-rs"
  exit 1
fi
graphify-rs --version
```

If the binary is found, print nothing extra and move straight to Step 2.

### Step 2 - Build the knowledge graph

Run the full pipeline. graphify-rs handles detection, extraction, building, clustering, analysis, and export in a single command:

```bash
graphify-rs build --path INPUT_PATH --output .graphify
```

Replace INPUT_PATH with the actual path the user provided.

Available flags:
- `--no-llm`: skip Claude API semantic extraction (AST-only, free, fast)
- `--code-only`: only process code files
- `--update`: incremental rebuild, only re-extract changed files
- `--format json,html,report,wiki,svg,graphml,cypher,obsidian`: select export formats (default: all)
- `--jobs N`: control parallelism
- `--max-viz-nodes N`: maximum nodes in HTML visualization (default: 2000, increase for larger projects)

The command outputs progress with a progress bar and colored status messages.

If the user specified `--code-only` or `--no-llm`, pass those flags through.

### Step 3 - Present results

After the build completes, read and present key sections from the report:

```bash
cat .graphify/GRAPH_REPORT.md
```

Present these sections directly in chat:
- God Nodes (highest-connectivity nodes)
- Surprising Connections (cross-community bridges)
- Suggested Questions

Do NOT paste the full report - just those three sections. Keep it concise.

### Step 4 - Offer to explore

Pick the single most interesting suggested question from the report and ask:

> "The most interesting question this graph can answer: **[question]**. Want me to trace it?"

If the user says yes, run:

```bash
graphify-rs query "QUESTION" --graph .graphify/graph.json
```

Walk them through the answer using the graph structure. Each answer should end with a natural follow-up so the session feels like navigation.

---

## Keeping the graph current after code changes

**This is critical for agentic workflows.** When you (or the user) modify code files during a session, the knowledge graph becomes stale. You MUST rebuild it to keep answers accurate.

### Rule: After modifying code, rebuild the graph

After you finish a batch of code changes (new files, edited functions, refactored modules), run:

```bash
graphify-rs build --path . --output .graphify --no-llm --update
```

- `--update`: only re-extract changed files (fast, uses SHA256 cache)
- `--no-llm`: skip Claude API (AST-only rebuild is free and fast, ~2-5s)
- This updates `graph.json`, `GRAPH_REPORT.md`, and all exports

### When to rebuild

- **After writing/editing code**: rebuild before answering architecture questions about the changed code
- **After refactoring**: rebuild so community structure reflects the new module boundaries
- **After adding new files**: rebuild to include them in the graph
- **NOT after every single edit**: batch changes, then rebuild once. Don't rebuild after a one-line typo fix unless the user asks architecture questions about it.

### Automated alternatives

Instead of manual rebuilds, the user can set up always-on monitoring:

1. **Watch mode** (background process):
   ```bash
   graphify-rs watch --path . --output .graphify
   ```
   Auto-rebuilds on file changes with 3s debounce. Best for long coding sessions.

2. **Git hooks** (per-commit):
   ```bash
   graphify-rs hook install
   ```
   Rebuilds after every `git commit`. No background process needed.

3. **Claude Code integration** (always-on):
   ```bash
   graphify-rs claude install
   ```
   Writes a PreToolUse hook to `.claude/settings.json` that reminds you to check the graph before searching files, plus a CLAUDE.md rule to rebuild after code changes.

---

## For /graphify query

```bash
graphify-rs query "QUESTION" --graph .graphify/graph.json
```

Add `--dfs` for depth-first traversal (trace specific paths). Add `--budget N` to control output size (default 2000 tokens).

After answering, save the result for the feedback loop:

```bash
graphify-rs save-result --question "QUESTION" --answer "ANSWER" --nodes NODE1 NODE2
```

---

## For /graphify add

Fetch a URL and add it to the corpus:

```bash
graphify-rs ingest URL --output .graphify
```

Then rebuild incrementally:

```bash
graphify-rs build --path . --output .graphify --update
```

---

## For --watch

Start a background watcher that monitors a folder and auto-updates the graph:

```bash
graphify-rs watch --path INPUT_PATH --output .graphify
```

Code changes trigger AST re-extraction + rebuild automatically (no LLM needed). Press Ctrl+C to stop.

---

## For MCP server

Start a stdio MCP server exposing 15 query tools:

```bash
graphify-rs serve --graph .graphify/graph.json
```

Start a local HTTP MCP server instead:

```bash
graphify-rs serve --transport http --http-bind 127.0.0.1:0 --registry-path .graphify/.graphifyq-server.json --graph .graphify/graph.json
```

For Codex-style short-lived calls, prefer `graphifyq`:

```bash
graphifyq ensure
graphifyq query "where is authentication wired?"
graphifyq stats
graphifyq summary architecture --budget 3000
```

`graphifyq` is intentionally like `fffq`: it starts/reuses a per-project local sidecar, stores the registry under `.graphify/.graphifyq-server.json`, and exits after printing the requested context.

Tools: `query_graph`, `get_node`, `get_neighbors`, `get_community`, `god_nodes`, `graph_stats`, `shortest_path`, `find_all_paths`, `weighted_path`, `community_bridges`, `graph_diff`, `pagerank`, `detect_cycles`, `smart_summary`, `find_similar`.

To configure in Claude Desktop, add to `claude_desktop_config.json`:
```json
{
  "mcpServers": {
    "graphify": {
      "command": "graphify-rs",
      "args": ["serve", "--graph", "/absolute/path/to/.graphify/graph.json"]
    }
  }
}
```

---

## For git commit hook

```bash
graphify-rs hook install     # install post-commit/post-checkout hooks
graphify-rs hook uninstall   # remove hooks
graphify-rs hook status      # check if hooks are installed
```

After every `git commit`, the hook auto-rebuilds the graph (code-only, no LLM).

---

## For Claude Code integration

Run once per project to make graphify always-on:

```bash
graphify-rs claude install     # write ## graphify section to CLAUDE.md + PreToolUse hook
graphify-rs claude uninstall   # remove the section
```

---

## Additional commands

```bash
graphify-rs stats .graphify/graph.json              # show graph statistics
graphify-rs diff old-graph.json new-graph.json         # compare two graph snapshots
graphify-rs benchmark .graphify/graph.json          # token efficiency benchmark
graphify-rs init                                       # create graphify.toml config file
graphify-rs completions bash                           # generate shell completions (bash/zsh/fish)
```

---

## Honesty Rules

- Never invent an edge. If unsure, use AMBIGUOUS.
- Never skip the corpus check warning.
- Always show token cost in the report.
- Never hide cohesion scores behind symbols - show the raw number.
- Never run HTML viz on a graph with more than 5,000 nodes without warning the user.
