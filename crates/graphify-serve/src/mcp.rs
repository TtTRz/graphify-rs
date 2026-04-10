//! MCP (Model Context Protocol) server implementation.
//!
//! Implements JSON-RPC 2.0 over stdio for AI coding assistant integration.
//! Protocol spec: <https://modelcontextprotocol.io/>

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, BufRead, Write};
use std::path::Path;

use graphify_core::graph::KnowledgeGraph;
use serde_json::{Value, json};
use tracing::{debug, error, info};

use crate::{ServeError, bfs, graph_stats, load_graph, score_nodes, subgraph_to_text};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const SERVER_NAME: &str = "graphify-rs";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "2024-11-05";

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn tool_definitions() -> Value {
    json!([
        {
            "name": "query_graph",
            "description": "Search the knowledge graph with a natural language question. Returns relevant nodes and relationships as structured context.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "Natural language question to search for"
                    },
                    "budget": {
                        "type": "number",
                        "description": "Token budget for response (default: 2000)",
                        "default": 2000
                    }
                },
                "required": ["question"]
            }
        },
        {
            "name": "get_node",
            "description": "Get details of a specific node by its ID, including label, type, source file, community, and neighbors.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": {
                        "type": "string",
                        "description": "The node ID to look up"
                    }
                },
                "required": ["node_id"]
            }
        },
        {
            "name": "get_neighbors",
            "description": "Get all neighbors of a node up to a given depth.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": {
                        "type": "string",
                        "description": "The node ID to get neighbors for"
                    },
                    "depth": {
                        "type": "number",
                        "description": "Traversal depth (default: 1)",
                        "default": 1
                    }
                },
                "required": ["node_id"]
            }
        },
        {
            "name": "get_community",
            "description": "Get all nodes belonging to a specific community.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "community_id": {
                        "type": "number",
                        "description": "The community ID"
                    }
                },
                "required": ["community_id"]
            }
        },
        {
            "name": "god_nodes",
            "description": "Get the most connected (highest degree) nodes in the graph.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "top_n": {
                        "type": "number",
                        "description": "Number of top nodes to return (default: 10)",
                        "default": 10
                    }
                }
            }
        },
        {
            "name": "graph_stats",
            "description": "Get overall graph statistics: node count, edge count, community count, degree stats.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "shortest_path",
            "description": "Find the shortest path between two nodes in the knowledge graph.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Source node ID"
                    },
                    "target": {
                        "type": "string",
                        "description": "Target node ID"
                    }
                },
                "required": ["source", "target"]
            }
        }
    ])
}

// ---------------------------------------------------------------------------
// JSON-RPC helpers
// ---------------------------------------------------------------------------

fn jsonrpc_response(id: &Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

fn jsonrpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

fn tool_result_text(text: &str) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": text
        }]
    })
}

fn tool_result_error(text: &str) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": text
        }],
        "isError": true
    })
}

// ---------------------------------------------------------------------------
// Tool handlers
// ---------------------------------------------------------------------------

fn handle_query_graph(graph: &KnowledgeGraph, args: &Value) -> Value {
    let question = args["question"].as_str().unwrap_or("");
    let budget = args["budget"].as_u64().unwrap_or(2000) as usize;

    if question.is_empty() {
        return tool_result_error("Missing required parameter: question");
    }

    let terms: Vec<String> = question
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .map(|w| w.to_lowercase())
        .collect();

    if terms.is_empty() {
        return tool_result_text("No meaningful search terms found in the question.");
    }

    let scored = score_nodes(graph, &terms);
    if scored.is_empty() {
        return tool_result_text("No matching nodes found for the given question.");
    }

    let start: Vec<String> = scored.iter().take(5).map(|(_, id)| id.clone()).collect();
    let (nodes, edges) = bfs(graph, &start, 2);
    let text = subgraph_to_text(graph, &nodes, &edges, budget);

    tool_result_text(&text)
}

fn handle_get_node(graph: &KnowledgeGraph, args: &Value) -> Value {
    let node_id = args["node_id"].as_str().unwrap_or("");
    if node_id.is_empty() {
        return tool_result_error("Missing required parameter: node_id");
    }

    match graph.get_node(node_id) {
        Some(node) => {
            let neighbors = graph.neighbor_ids(node_id);
            let degree = graph.degree(node_id);
            let result = json!({
                "id": node.id,
                "label": node.label,
                "node_type": node.node_type,
                "source_file": node.source_file,
                "source_location": node.source_location,
                "community": node.community,
                "degree": degree,
                "neighbors": neighbors,
            });
            tool_result_text(&serde_json::to_string_pretty(&result).unwrap_or_default())
        }
        None => tool_result_error(&format!("Node not found: {node_id}")),
    }
}

fn handle_get_neighbors(graph: &KnowledgeGraph, args: &Value) -> Value {
    let node_id = args["node_id"].as_str().unwrap_or("");
    let depth = args["depth"].as_u64().unwrap_or(1) as usize;

    if node_id.is_empty() {
        return tool_result_error("Missing required parameter: node_id");
    }

    if graph.get_node(node_id).is_none() {
        return tool_result_error(&format!("Node not found: {node_id}"));
    }

    let (nodes, edges) = bfs(graph, &[node_id.to_string()], depth);

    let mut neighbor_info: Vec<Value> = Vec::new();
    for nid in &nodes {
        if nid == node_id {
            continue; // skip the start node
        }
        if let Some(node) = graph.get_node(nid) {
            // Find edges connecting to this neighbor
            let edge_relations: Vec<&str> = edges
                .iter()
                .filter(|(s, t)| (s == node_id && t == nid) || (s == nid && t == node_id))
                .map(|_| "connected")
                .collect();

            neighbor_info.push(json!({
                "id": node.id,
                "label": node.label,
                "node_type": node.node_type,
                "source_file": node.source_file,
                "community": node.community,
                "edge_count": edge_relations.len(),
            }));
        }
    }

    let result = json!({
        "node_id": node_id,
        "depth": depth,
        "neighbor_count": neighbor_info.len(),
        "neighbors": neighbor_info,
    });

    tool_result_text(&serde_json::to_string_pretty(&result).unwrap_or_default())
}

fn handle_get_community(graph: &KnowledgeGraph, args: &Value) -> Value {
    let community_id = match args["community_id"].as_u64() {
        Some(id) => id as usize,
        None => return tool_result_error("Missing required parameter: community_id"),
    };

    let mut members: Vec<Value> = Vec::new();
    for node_id in graph.node_ids() {
        if let Some(node) = graph.get_node(&node_id)
            && node.community == Some(community_id)
        {
            members.push(json!({
                "id": node.id,
                "label": node.label,
                "node_type": node.node_type,
                "source_file": node.source_file,
                "degree": graph.degree(&node_id),
            }));
        }
    }

    if members.is_empty() {
        return tool_result_error(&format!("Community not found or empty: {community_id}"));
    }

    // Sort by degree descending
    members.sort_by(|a, b| {
        let da = a["degree"].as_u64().unwrap_or(0);
        let db = b["degree"].as_u64().unwrap_or(0);
        db.cmp(&da)
    });

    let result = json!({
        "community_id": community_id,
        "member_count": members.len(),
        "members": members,
    });

    tool_result_text(&serde_json::to_string_pretty(&result).unwrap_or_default())
}

fn handle_god_nodes(graph: &KnowledgeGraph, args: &Value) -> Value {
    let top_n = args["top_n"].as_u64().unwrap_or(10) as usize;

    let gods = graphify_analyze::god_nodes(graph, top_n);

    let result: Vec<Value> = gods
        .iter()
        .enumerate()
        .map(|(i, g)| {
            json!({
                "rank": i + 1,
                "id": g.id,
                "label": g.label,
                "degree": g.degree,
                "community": g.community,
            })
        })
        .collect();

    let output = json!({
        "top_n": top_n,
        "god_nodes": result,
    });

    tool_result_text(&serde_json::to_string_pretty(&output).unwrap_or_default())
}

fn handle_graph_stats(graph: &KnowledgeGraph) -> Value {
    let stats = graph_stats(graph);
    tool_result_text(&serde_json::to_string_pretty(&stats).unwrap_or_default())
}

fn handle_shortest_path(graph: &KnowledgeGraph, args: &Value) -> Value {
    let source = args["source"].as_str().unwrap_or("");
    let target = args["target"].as_str().unwrap_or("");

    if source.is_empty() || target.is_empty() {
        return tool_result_error("Missing required parameters: source and target");
    }

    if graph.get_node(source).is_none() {
        return tool_result_error(&format!("Source node not found: {source}"));
    }
    if graph.get_node(target).is_none() {
        return tool_result_error(&format!("Target node not found: {target}"));
    }

    if source == target {
        let node = graph.get_node(source).unwrap();
        let result = json!({
            "source": source,
            "target": target,
            "path_length": 0,
            "path": [{"id": node.id, "label": node.label}],
        });
        return tool_result_text(&serde_json::to_string_pretty(&result).unwrap_or_default());
    }

    // BFS shortest path
    let mut visited: HashSet<String> = HashSet::new();
    let mut parent: HashMap<String, String> = HashMap::new();
    let mut queue: VecDeque<String> = VecDeque::new();

    visited.insert(source.to_string());
    queue.push_back(source.to_string());

    let mut found = false;
    while let Some(current) = queue.pop_front() {
        if current == target {
            found = true;
            break;
        }
        for neighbor in graph.neighbor_ids(&current) {
            if !visited.contains(&neighbor) {
                visited.insert(neighbor.clone());
                parent.insert(neighbor.clone(), current.clone());
                queue.push_back(neighbor);
            }
        }
    }

    if !found {
        return tool_result_text(&format!(
            "No path found between '{source}' and '{target}'. They may be in disconnected components."
        ));
    }

    // Reconstruct path
    let mut path = vec![target.to_string()];
    let mut current = target.to_string();
    while let Some(p) = parent.get(&current) {
        path.push(p.clone());
        current = p.clone();
    }
    path.reverse();

    let path_nodes: Vec<Value> = path
        .iter()
        .map(|id| {
            let label = graph.get_node(id).map(|n| n.label.as_str()).unwrap_or(id);
            json!({"id": id, "label": label})
        })
        .collect();

    let result = json!({
        "source": source,
        "target": target,
        "path_length": path.len() - 1,
        "path": path_nodes,
    });

    tool_result_text(&serde_json::to_string_pretty(&result).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Request dispatcher
// ---------------------------------------------------------------------------

fn dispatch_tools_call(graph: &KnowledgeGraph, request: &Value) -> Value {
    let id = &request["id"];
    let tool_name = request["params"]["name"].as_str().unwrap_or("");
    let args = &request["params"]["arguments"];

    debug!("tools/call: {tool_name}");

    let result = match tool_name {
        "query_graph" => handle_query_graph(graph, args),
        "get_node" => handle_get_node(graph, args),
        "get_neighbors" => handle_get_neighbors(graph, args),
        "get_community" => handle_get_community(graph, args),
        "god_nodes" => handle_god_nodes(graph, args),
        "graph_stats" => handle_graph_stats(graph),
        "shortest_path" => handle_shortest_path(graph, args),
        _ => tool_result_error(&format!("Unknown tool: {tool_name}")),
    };

    jsonrpc_response(id, result)
}

fn dispatch(graph: &KnowledgeGraph, request: &Value) -> Option<Value> {
    let method = request["method"].as_str().unwrap_or("");
    let id = &request["id"];

    match method {
        "initialize" => {
            info!("MCP initialize");
            Some(jsonrpc_response(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": SERVER_NAME,
                        "version": SERVER_VERSION
                    }
                }),
            ))
        }
        "notifications/initialized" => {
            // Notification — no response needed
            debug!("Client initialized");
            None
        }
        "tools/list" => {
            debug!("tools/list");
            Some(jsonrpc_response(
                id,
                json!({
                    "tools": tool_definitions()
                }),
            ))
        }
        "tools/call" => Some(dispatch_tools_call(graph, request)),
        "ping" => Some(jsonrpc_response(id, json!({}))),
        _ => {
            // Unknown method — return error if it has an id (i.e. it's a request, not a notification)
            if id.is_null() {
                None // notification, ignore
            } else {
                Some(jsonrpc_error(
                    id,
                    -32601,
                    &format!("Method not found: {method}"),
                ))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Server entry point
// ---------------------------------------------------------------------------

/// Start the MCP server, reading JSON-RPC requests from stdin and writing
/// responses to stdout. Logs go to stderr so they don't interfere with the
/// protocol.
pub fn run_mcp_server(graph_path: &Path) -> Result<(), ServeError> {
    // Redirect tracing to stderr (already the default for tracing_subscriber)
    let graph = load_graph(graph_path)?;
    let stats = crate::graph_stats(&graph);
    let null = Value::Null;
    info!(
        "MCP server started: {} nodes, {} edges",
        stats.get("node_count").unwrap_or(&null),
        stats.get("edge_count").unwrap_or(&null),
    );

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                error!("stdin read error: {e}");
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                error!("JSON parse error: {e}");
                let err = jsonrpc_error(&Value::Null, -32700, &format!("Parse error: {e}"));
                let _ = writeln!(stdout_lock, "{}", serde_json::to_string(&err).unwrap());
                let _ = stdout_lock.flush();
                continue;
            }
        };

        if let Some(response) = dispatch(&graph, &request) {
            let out = serde_json::to_string(&response).unwrap_or_default();
            if let Err(e) = writeln!(stdout_lock, "{}", out) {
                error!("stdout write error: {e}");
                break;
            }
            let _ = stdout_lock.flush();
        }
    }

    info!("MCP server shutting down");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::confidence::Confidence;
    use graphify_core::model::{GraphEdge, GraphNode, NodeType};

    fn make_node(id: &str, label: &str, community: Option<usize>) -> GraphNode {
        GraphNode {
            id: id.into(),
            label: label.into(),
            source_file: "test.rs".into(),
            source_location: None,
            node_type: NodeType::Class,
            community,
            extra: HashMap::new(),
        }
    }

    fn make_edge(src: &str, tgt: &str) -> GraphEdge {
        GraphEdge {
            source: src.into(),
            target: tgt.into(),
            relation: "calls".into(),
            confidence: Confidence::Extracted,
            confidence_score: 1.0,
            source_file: "test.rs".into(),
            source_location: None,
            weight: 1.0,
            extra: HashMap::new(),
        }
    }

    fn test_graph() -> KnowledgeGraph {
        let mut g = KnowledgeGraph::new();
        g.add_node(make_node("auth", "AuthService", Some(0)))
            .unwrap();
        g.add_node(make_node("user", "UserManager", Some(0)))
            .unwrap();
        g.add_node(make_node("db", "Database", Some(1))).unwrap();
        g.add_node(make_node("cache", "CacheLayer", Some(1)))
            .unwrap();
        g.add_edge(make_edge("auth", "user")).unwrap();
        g.add_edge(make_edge("auth", "db")).unwrap();
        g.add_edge(make_edge("user", "db")).unwrap();
        g.add_edge(make_edge("user", "cache")).unwrap();
        g
    }

    #[test]
    fn test_initialize() {
        let g = test_graph();
        let req = json!({"jsonrpc": "2.0", "method": "initialize", "id": 1});
        let resp = dispatch(&g, &req).unwrap();
        assert_eq!(resp["id"], 1);
        assert!(resp["result"]["protocolVersion"].is_string());
        assert!(resp["result"]["capabilities"]["tools"].is_object());
        assert_eq!(resp["result"]["serverInfo"]["name"], SERVER_NAME);
    }

    #[test]
    fn test_tools_list() {
        let g = test_graph();
        let req = json!({"jsonrpc": "2.0", "method": "tools/list", "id": 2});
        let resp = dispatch(&g, &req).unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 7);

        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"query_graph"));
        assert!(names.contains(&"get_node"));
        assert!(names.contains(&"get_neighbors"));
        assert!(names.contains(&"get_community"));
        assert!(names.contains(&"god_nodes"));
        assert!(names.contains(&"graph_stats"));
        assert!(names.contains(&"shortest_path"));
    }

    #[test]
    fn test_query_graph() {
        let g = test_graph();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 3,
            "params": {"name": "query_graph", "arguments": {"question": "auth service"}}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Knowledge Graph Context"));
        assert!(text.contains("AuthService"));
    }

    #[test]
    fn test_get_node() {
        let g = test_graph();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 4,
            "params": {"name": "get_node", "arguments": {"node_id": "auth"}}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("AuthService"));
        assert!(text.contains("\"degree\""));
    }

    #[test]
    fn test_get_node_not_found() {
        let g = test_graph();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 5,
            "params": {"name": "get_node", "arguments": {"node_id": "nonexistent"}}
        });
        let resp = dispatch(&g, &req).unwrap();
        assert!(resp["result"]["isError"].as_bool().unwrap_or(false));
    }

    #[test]
    fn test_get_neighbors() {
        let g = test_graph();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 6,
            "params": {"name": "get_neighbors", "arguments": {"node_id": "auth", "depth": 1}}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("neighbor_count"));
    }

    #[test]
    fn test_get_community() {
        let g = test_graph();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 7,
            "params": {"name": "get_community", "arguments": {"community_id": 0}}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("AuthService") || text.contains("UserManager"));
    }

    #[test]
    fn test_god_nodes() {
        let g = test_graph();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 8,
            "params": {"name": "god_nodes", "arguments": {"top_n": 3}}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("god_nodes"));
    }

    #[test]
    fn test_graph_stats() {
        let g = test_graph();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 9,
            "params": {"name": "graph_stats", "arguments": {}}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("node_count"));
        assert!(text.contains("edge_count"));
    }

    #[test]
    fn test_shortest_path() {
        let g = test_graph();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 10,
            "params": {"name": "shortest_path", "arguments": {"source": "auth", "target": "cache"}}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("path_length"));
        // auth -> user -> cache = length 2
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["path_length"], 2);
    }

    #[test]
    fn test_shortest_path_no_path() {
        let mut g = KnowledgeGraph::new();
        g.add_node(make_node("a", "A", None)).unwrap();
        g.add_node(make_node("b", "B", None)).unwrap();
        // No edge between them
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 11,
            "params": {"name": "shortest_path", "arguments": {"source": "a", "target": "b"}}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("No path found"));
    }

    #[test]
    fn test_shortest_path_same_node() {
        let g = test_graph();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 12,
            "params": {"name": "shortest_path", "arguments": {"source": "auth", "target": "auth"}}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["path_length"], 0);
    }

    #[test]
    fn test_unknown_tool() {
        let g = test_graph();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 13,
            "params": {"name": "nonexistent_tool", "arguments": {}}
        });
        let resp = dispatch(&g, &req).unwrap();
        assert!(resp["result"]["isError"].as_bool().unwrap_or(false));
    }

    #[test]
    fn test_unknown_method() {
        let g = test_graph();
        let req = json!({"jsonrpc": "2.0", "method": "unknown/method", "id": 14});
        let resp = dispatch(&g, &req).unwrap();
        assert!(resp["error"].is_object());
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[test]
    fn test_notification_no_response() {
        let g = test_graph();
        let req = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        assert!(dispatch(&g, &req).is_none());
    }

    #[test]
    fn test_ping() {
        let g = test_graph();
        let req = json!({"jsonrpc": "2.0", "method": "ping", "id": 15});
        let resp = dispatch(&g, &req).unwrap();
        assert_eq!(resp["id"], 15);
        assert!(resp["result"].is_object());
    }
}
