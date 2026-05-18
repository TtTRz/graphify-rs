# Extraction & Quality Improvements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Improve call-graph accuracy for regex extractors, reduce code duplication, add cross-file call resolution, add MCP pagination, eliminate production unwrap panics, and split the oversized html.rs.

**Architecture:** Six independent improvements across extraction, serving, and export crates. Each task is self-contained and can be implemented and tested independently. No cross-task dependencies.

**Tech Stack:** Rust, tree-sitter, regex, serde_json

---

### Task 1: Fix regex extractor call-graph false positives

**Files:**
- Modify: `crates/graphify-extract/src/ast_extract/mod.rs:246-277`
- Test: `crates/graphify-extract/tests/ast_extract.rs`

The regex `infer_calls` function matches `\bfunc_name\s*\(` which causes false positives: `v.get(0)` matches `get()`, `self.target()` matches `target()`. Fix by requiring the name to appear at the start of a statement or after `=`, `(`, `,`, or whitespace with no preceding `.`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn regex_call_graph_no_false_positive_via_dot() {
    // regex-based extractor (e.g. Kotlin via generic)
    let source = r#"
fun target() {}
fun caller() {
    val v = listOf(1, 2, 3)
    v.get(0)
    target()
}
"#;
    let result = extract(Path::new("test.kt"), source.as_bytes(), "kotlin").unwrap();
    let call_edges: Vec<_> = result.edges.iter().filter(|e| e.relation == "calls").collect();
    // v.get(0) should NOT match the "get" function; only target() should
    assert_eq!(call_edges.len(), 1, "expected exactly 1 call edge, got {:?}", call_edges);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p graphify-extract regex_call_graph_no_false_positive_via_dot`
Expected: FAIL (currently `v.get(0)` produces a spurious edge)

- [ ] **Step 3: Fix `infer_calls` to reject dot-prefixed calls**

Replace the current pattern:
```rust
let pattern = format!(r"\b{}\s*\(", regex::escape(callee_name));
```
With a negative lookbehind to reject `.func_name(`:
```rust
let pattern = format!(r"(?<!\.){}\s*\(", regex::escape(callee_name));
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p graphify-extract regex_call_graph_no_false_positive_via_dot`
Expected: PASS

- [ ] **Step 5: Run full test suite**

Run: `cargo test -p graphify-extract`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/graphify-extract/src/ast_extract/mod.rs crates/graphify-extract/tests/ast_extract.rs
git commit -m "fix: regex call-graph false positives from dot-prefixed method calls"
```

---

### Task 2: Extract Elixir dispatch logic in walk_node

**Files:**
- Modify: `crates/graphify-extract/src/treesitter/mod.rs:240-310`

The `walk_node` function has three blocks with `if ctx.lang == "elixir" && kind == "call"` for imports, classes, and functions respectively. Extract the Elixir call classification into a single helper.

- [ ] **Step 1: Write the helper function**

In `mod.rs`, add before `walk_node`:

```rust
enum ElixirCallKind {
    Import,
    Class,
    Function,
    Other,
}

fn classify_elixir_call(node: Node, source: &[u8], config: &TsConfig) -> ElixirCallKind {
    let target = node
        .child_by_field_name(config.name_field)
        .map(|n| node_text(n, source))
        .unwrap_or_default();
    match target.as_str() {
        "import" | "use" | "require" | "alias" => ElixirCallKind::Import,
        "defmodule" | "defprotocol" | "defimpl" => ElixirCallKind::Class,
        "def" | "defp" | "defmacro" | "defmacrop" | "defguard" | "defguardp" | "defdelegate" => {
            ElixirCallKind::Function
        }
        _ => ElixirCallKind::Other,
    }
}
```

- [ ] **Step 2: Refactor walk_node to use the helper**

Replace the three Elixir-specific blocks with:
```rust
if config.import_types.contains(kind) {
    if ctx.lang == "elixir" && kind == "call" {
        match classify_elixir_call(node, source, config) {
            ElixirCallKind::Import => {
                imports::extract_import(node, source, ctx.file_nid, ctx.str_path, ctx.lang, ctx.edges, ctx.nodes);
                return;
            }
            _ => {} // fall through to class/function checks below
        }
    } else if ctx.lang == "ruby" && kind == "call" {
        // existing ruby logic unchanged
        ...
    } else {
        imports::extract_import(node, source, ctx.file_nid, ctx.str_path, ctx.lang, ctx.edges, ctx.nodes);
        return;
    }
}

if config.class_types.contains(kind) {
    if ctx.lang == "elixir" && kind == "call" {
        if matches!(classify_elixir_call(node, source, config), ElixirCallKind::Class) {
            handlers::handle_class_like(node, source, config, ctx);
            return;
        }
    } else {
        handlers::handle_class_like(node, source, config, ctx);
        return;
    }
}

if config.function_types.contains(kind) {
    if ctx.lang == "elixir" && kind == "call" {
        if matches!(classify_elixir_call(node, source, config), ElixirCallKind::Function) {
            handlers::handle_function(node, source, config, ctx, parent_class_nid);
            return;
        }
    } else {
        handlers::handle_function(node, source, config, ctx, parent_class_nid);
        return;
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --workspace`
Expected: All 437+ tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/graphify-extract/src/treesitter/mod.rs
git commit -m "refactor: extract Elixir call classification into classify_elixir_call helper"
```

---

### Task 3: Cross-file call resolution

**Files:**
- Modify: `crates/graphify-extract/src/treesitter/mod.rs` (call-graph pass)
- Modify: `crates/graphify-extract/src/lib.rs:269-470` (`resolve_cross_file_imports`)
- Test: `crates/graphify-extract/tests/treesitter.rs`
- Test: `crates/graphify-extract/tests/cross_file.rs`

After `resolve_cross_file_imports` runs, we know which files import which modules (via "imports" edges). When a function in file A calls a function defined in imported file B, we should create a cross-file "calls" edge. Currently call-graph only matches within the same file.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn ts_cross_file_call_resolution() {
    // Simulate two Rust files extracted independently then combined
    let source_a = br#"
fn helper() {}
"#;
    let source_b = br#"
fn main() {
    helper();
}
"#;
    let mut result_a = try_extract(Path::new("src/a.rs"), source_a, "rust").unwrap();
    let result_b = try_extract(Path::new("src/b.rs"), source_b, "rust").unwrap();

    // Merge results
    result_a.nodes.extend(result_b.nodes);
    result_a.edges.extend(result_b.edges);

    // Before fix: no cross-file call edge exists
    // After fix: resolve_cross_file_calls should find that main() calls helper()
    // even though they are in different files
    let cross_calls: Vec<_> = result_a.edges.iter()
        .filter(|e| e.relation == "calls")
        .collect();
    // At minimum, within b.rs, main() should call helper()
    // (if helper is in the label_to_nid map)
    assert!(!cross_calls.is_empty(), "should detect cross-file call");
}
```

- [ ] **Step 2: Add `resolve_cross_file_calls` function**

In `lib.rs`, add a new function after `resolve_cross_file_imports`:

```rust
fn resolve_cross_file_calls(result: &mut ExtractionResult) {
    let label_to_nid: HashMap<String, String> = result
        .nodes
        .iter()
        .filter(|n| matches!(n.node_type, NodeType::Function | NodeType::Method))
        .map(|n| {
            let normalized = n.label.trim_end_matches("()").trim_start_matches('.').to_lowercase();
            (normalized, n.id.clone())
        })
        .collect();

    let mut seen: HashSet<(String, String)> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    let existing_calls: Vec<(String, String)> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    for (caller_nid, _) in &existing_calls {
        let caller = match result.nodes.iter().find(|n| &n.id == caller_nid) {
            Some(n) => n,
            None => continue,
        };
        // Find import edges from the caller's file
        let caller_file = &caller.source_file;
        let imported_targets: Vec<String> = result
            .edges
            .iter()
            .filter(|e| {
                e.relation == "imports"
                    && result.nodes.iter().any(|n| n.id == e.source && n.source_file == *caller_file)
            })
            .map(|e| e.target.clone())
            .collect();

        // For each imported file, check if the callee name matches a function there
        for import_target in &imported_targets {
            if let Some(callee_nid) = label_to_nid.get(import_target.to_lowercase().as_str()) {
                let key = (caller_nid.clone(), callee_nid.clone());
                if seen.insert(key) {
                    result.edges.push(GraphEdge {
                        source: caller_nid.clone(),
                        target: callee_nid.clone(),
                        relation: "calls".to_string(),
                        confidence: Confidence::Inferred,
                        confidence_score: Confidence::Inferred.default_score(),
                        source_file: caller_file.clone(),
                        source_location: None,
                        weight: 0.5,
                        extra: HashMap::new(),
                    });
                }
            }
        }
    }
}
```

- [ ] **Step 3: Call it from the extraction pipeline**

In `lib.rs` `extract` function, after `resolve_cross_file_imports(&mut combined);`, add:
```rust
resolve_cross_file_calls(&mut combined);
```

- [ ] **Step 4: Run tests and adjust**

Run: `cargo test -p graphify-extract`
Adjust test expectations and function logic as needed. The initial implementation may need refinement based on actual import edge structure.

- [ ] **Step 5: Run full workspace tests**

Run: `cargo test --workspace`

- [ ] **Step 6: Commit**

```bash
git add crates/graphify-extract/src/lib.rs crates/graphify-extract/tests/
git commit -m "feat: cross-file call resolution via import edge matching"
```

---

### Task 4: MCP tool pagination

**Files:**
- Modify: `crates/graphify-serve/src/mcp/handlers.rs:142-181` (`handle_get_community`)
- Modify: `crates/graphify-serve/src/mcp/handlers.rs:69-93` (`handle_get_node` → `handle_get_neighbors`)
- Modify: `crates/graphify-serve/src/mcp/tools.rs` (tool definitions)

Add `limit` and `offset` parameters to `get_community` and `get_neighbors` so LLMs don't receive thousands of entries at once.

- [ ] **Step 1: Add pagination to `handle_get_community`**

```rust
pub(crate) fn handle_get_community(graph: &KnowledgeGraph, args: &Value) -> Value {
    let community_id = match args["community_id"].as_u64() {
        Some(id) => id as usize,
        None => return tool_result_error("Missing required parameter: community_id"),
    };
    let limit = args["limit"].as_u64().unwrap_or(50) as usize;
    let offset = args["offset"].as_u64().unwrap_or(0) as usize;

    let mut members: Vec<Value> = Vec::new();
    // ... collect members as before ...

    let total = members.len();
    members.sort_by(|a, b| { /* sort by degree desc as before */ });

    let paginated: Vec<_> = members.into_iter().skip(offset).take(limit).collect();

    let result = json!({
        "community_id": community_id,
        "total_members": total,
        "offset": offset,
        "limit": limit,
        "returned": paginated.len(),
        "members": paginated,
    });
    tool_result_json(&result)
}
```

- [ ] **Step 2: Add pagination to `handle_get_neighbors`**

```rust
let limit = args["limit"].as_u64().unwrap_or(50) as usize;
let offset = args["offset"].as_u64().unwrap_or(0) as usize;
// ... after building neighbor_info ...
let total = neighbor_info.len();
let paginated: Vec<_> = neighbor_info.into_iter().skip(offset).take(limit).collect();
```

- [ ] **Step 3: Update tool definitions in tools.rs**

Add `limit` and `offset` to the `inputSchema` properties for `get_community` and `get_neighbors`:

```json
"limit": {
    "type": "integer",
    "description": "Maximum number of results to return (default: 50)",
    "default": 50
},
"offset": {
    "type": "integer",
    "description": "Number of results to skip (default: 0)",
    "default": 0
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --workspace`

- [ ] **Step 5: Commit**

```bash
git add crates/graphify-serve/src/mcp/handlers.rs crates/graphify-serve/src/mcp/tools.rs
git commit -m "feat: add limit/offset pagination to get_community and get_neighbors MCP tools"
```

---

### Task 5: Eliminate production unwrap in export crates

**Files:**
- Modify: `crates/graphify-export/src/graphml.rs`
- Modify: `crates/graphify-export/src/cypher.rs`

Replace `.unwrap()` calls in production code paths with `?` or `map_err`, converting panics into proper error propagation.

- [ ] **Step 1: Refactor `graphml.rs`**

The `export_graphml` function returns `anyhow::Result<PathBuf>`. All `.unwrap()` calls on `write!` and `writeln!` macros (which return `std::fmt::Result`) should use `?` since `std::fmt::Error` can be converted via `anyhow`. Pattern:

```rust
// Before
writeln!(xml, r#"<key id="..." .../>"#).unwrap();
// After
writeln!(xml, r#"<key id="..." .../>"#)?;
```

Also handle the `BufWriter::into_inner().unwrap()` at the end:
```rust
// Before
writer.into_inner().unwrap().flush().unwrap();
// After
let buf = writer.into_inner().map_err(|e| anyhow::anyhow!("flush error: {}", e))?;
buf.flush()?;
```

- [ ] **Step 2: Refactor `cypher.rs`**

Same pattern — replace all `write!`/`writeln!` `.unwrap()` with `?`, and handle `BufWriter::into_inner()` properly.

For `ids.pop().unwrap()` in `build_unique_var_names`, the `unwrap` is safe because we only enter the for-loop body when the `ids` Vec is non-empty (the HashMap entry exists). Add a comment explaining safety, or replace with `if let Some(id) = ids.pop()` for defensive coding:

```rust
for (sanitized, mut ids) in name_to_ids {
    if let Some(primary_id) = ids.pop() {
        result.insert(primary_id, sanitized);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --workspace`

- [ ] **Step 4: Commit**

```bash
git add crates/graphify-export/src/graphml.rs crates/graphify-export/src/cypher.rs
git commit -m "fix: replace production unwrap with error propagation in export crates"
```

---

### Task 6: Split html.rs into html.rs + html_templates.rs

**Files:**
- Create: `crates/graphify-export/src/html_templates.rs`
- Modify: `crates/graphify-export/src/html.rs`
- Modify: `crates/graphify-export/src/lib.rs` (if needed for module registration)

The `build_html_template` function (lines 231-415) is 185 lines of raw HTML string interpolation. Move it to a separate file.

- [ ] **Step 1: Create `html_templates.rs`**

Extract `build_html_template` and the helper functions it uses (`escape_js`, `escape_html`) into the new file:

```rust
// crates/graphify-export/src/html_templates.rs

pub(crate) fn build_html_template(
    nodes_json: &str,
    edges_json: &str,
    community_json: &str,
    // ... other params
) -> String {
    // ... existing template code ...
}

pub(crate) fn escape_js(s: &str) -> String { /* ... */ }
pub(crate) fn escape_html(s: &str) -> String { /* ... */ }
```

- [ ] **Step 2: Update `html.rs`**

Replace the extracted functions with imports:
```rust
mod html_templates;
use html_templates::{build_html_template, escape_js, escape_html};
```

Remove the original function definitions from `html.rs`.

- [ ] **Step 3: Run tests**

Run: `cargo test --workspace`

- [ ] **Step 4: Commit**

```bash
git add crates/graphify-export/src/html.rs crates/graphify-export/src/html_templates.rs
git commit -m "refactor: extract HTML templates from html.rs into html_templates.rs"
```

---

## Execution Order

Tasks are independent and can be executed in any order. Recommended order by impact:

1. **Task 1** (regex call-graph fix) — quick, high impact on accuracy
2. **Task 5** (unwrap cleanup) — safety improvement, straightforward
3. **Task 4** (MCP pagination) — user-facing improvement
4. **Task 2** (Elixir refactor) — code quality
5. **Task 6** (html.rs split) — code organization
6. **Task 3** (cross-file calls) — most complex, needs careful testing

## Risks

- **Task 3** (cross-file calls): The initial implementation may create too many edges or false positives. The `weight: 0.5` and `Confidence::Inferred` should mitigate this, but may need tuning based on real-world results.
- **Task 1** (regex lookbehind): `(?<!\.)` is supported in Rust's `regex` crate (which uses the `fancy-regex` feature for lookbehind). If not available, an alternative is to match `\b` and then verify no preceding `.` in the matched text.
