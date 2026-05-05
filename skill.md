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
/graphify <path> --no-llm                             # no new LLM calls; preserve cached LLM output
/graphify <path> --llm-command "graphify-llm-codex --model gpt-5.4-mini" --llm-provider codex-cli  # enrich docs via installed Codex CLI
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
graphifyq ensure                                      # build graph + local Model2Vec semantic index; auto-refresh stale graph every 300s; start/reuse HTTP MCP sidecar
graphifyq ensure --with-llm --llm-command "graphify-llm-codex --model gpt-5.4-mini" --llm-provider codex-cli  # explicit LLM refresh
graphifyq query "how does auth work?"                 # short-lived semantic HTTP query helper
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
graphify-rs build --path INPUT_PATH --output .graphify --no-llm --embed
```

Replace INPUT_PATH with the actual path the user provided.

Available flags:
- `--no-llm`: make no new LLM calls; local AST + Markdown/RST/text document context still runs, and existing cached LLM output is preserved
- `--llm-command "graphify-llm-codex --model gpt-5.4-mini"`: enrich docs/prose through an installed Codex CLI instead of asking for an API key
- `--llm-provider codex-cli`: label/cache the external LLM backend so stale output is detected correctly
- `--code-only`: only process code files
- `--update`: incremental rebuild, only re-extract changed files
- `--format json,html,report,context,wiki,svg,graphml,cypher,obsidian`: select export formats (default: all)
- `--jobs N`: control parallelism
- `--max-viz-nodes N`: maximum nodes in HTML visualization (default: 2000, increase for larger projects)
- graphify respects root `.gitignore`, `.git/info/exclude`, and `.graphifyignore`; use `.graphifyignore` `!path` rules to re-include gitignored files for graphing
- `--embedding-provider model2vec|ollama|voyage`: choose semantic index backend; default is local Model2Vec. Ollama uses `/api/embed`; Voyage needs `VOYAGE_API_KEY`
- `--no-embed`: for `graphifyq`, opt out of the default semantic index when startup must be strictly AST-only/offline

LLM enrichment rules for agents:
- Default to `--no-llm` / `graphifyq ensure` unless the user explicitly asks for LLM enrichment.
- `--no-llm` must not be treated as "delete LLM output"; graphify preserves cached LLM annotations in `.graphify/llm-cache.json`.
- When running with `--llm-command`, graphify passes previous extraction back into the prompt for changed files, so follow-up builds are incremental/reiterative.
- If the LLM provider/command/prompt contract changes, old LLM output is marked stale-preserved rather than silently overwritten.
- Pass the repo or requested corpus root to `--path`; use `.graphifyignore` to narrow/re-include `src`, `bin`, docs, or generated directories.

The command outputs progress with a progress bar and colored status messages.

Keep `--no-llm --embed` by default. Default builds must not ask for Anthropic keys. If the user explicitly asks for legacy Claude document extraction, add `--anthropic-semantic` and require `ANTHROPIC_API_KEY`. If they ask for Ollama/Voyage embeddings, pass `--embedding-provider ollama|voyage` and the requested model.

### Step 3 - Present results

After the build completes, read and present key sections from the report:

```bash
cat .graphify/LLM_CONTEXT.md
printf "\n--- report excerpt ---\n"
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

After you finish a batch of code changes (new files, edited functions, refactored modules), prefer:

```bash
graphifyq ensure
```

`graphifyq` auto-refreshes stale per-repo graphs every 300s with a safe incremental build and restarts its local HTTP sidecar when needed.
If you need to force the refresh immediately, run:

```bash
graphify-rs build --path . --output .graphify --no-llm --update --embed
```

- `--update`: scans the full current file set but only re-extracts changed file contents via SHA256 cache, keeping `graph.json` complete
- `--no-llm`: skip Claude API; `--embed` keeps local semantic search current without an API key
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

Start a stdio MCP server exposing 16 query tools:

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
graphifyq ensure --no-embed      # opt out only for strict AST-only/offline startup
graphifyq query "where is authentication wired?"
graphifyq query --no-embed "where is queue backpressure handled?"
graphifyq stats
graphifyq summary architecture --budget 3000
```

`graphifyq` is intentionally like `fffq`: it starts/reuses a per-project local sidecar, stores the registry under `.graphify/.graphifyq-server.json`, and exits after printing the requested context.

Tools: `query_graph`, `semantic_query`, `get_node`, `get_neighbors`, `get_community`, `god_nodes`, `graph_stats`, `shortest_path`, `find_all_paths`, `weighted_path`, `community_bridges`, `graph_diff`, `pagerank`, `detect_cycles`, `smart_summary`, `find_similar`.

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
