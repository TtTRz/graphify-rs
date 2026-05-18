//! MCP (Model Context Protocol) server implementation.
//!
//! Implements JSON-RPC 2.0 over stdio for AI coding assistant integration.
//! Protocol spec: <https://modelcontextprotocol.io/>

mod handlers;
mod tools;

use std::io::{self, BufRead, Write};
use std::path::Path;

use graphify_core::graph::KnowledgeGraph;
use serde_json::{Value, json};
use tracing::{debug, error, info};

use crate::ServeError;

const SERVER_NAME: &str = "graphify-rs";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "2024-11-05";

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

fn dispatch_tools_call(graph: &KnowledgeGraph, request: &Value) -> Value {
    let id = &request["id"];
    let tool_name = request["params"]["name"].as_str().unwrap_or("");
    let args = &request["params"]["arguments"];

    debug!("tools/call: {tool_name}");

    let result = match tool_name {
        "query_graph" => handlers::handle_query_graph(graph, args),
        "get_node" => handlers::handle_get_node(graph, args),
        "get_neighbors" => handlers::handle_get_neighbors(graph, args),
        "get_community" => handlers::handle_get_community(graph, args),
        "god_nodes" => handlers::handle_god_nodes(graph, args),
        "graph_stats" => handlers::handle_graph_stats(graph),
        "shortest_path" => handlers::handle_shortest_path(graph, args),
        "find_all_paths" => handlers::handle_find_all_paths(graph, args),
        "weighted_path" => handlers::handle_weighted_path(graph, args),
        "community_bridges" => handlers::handle_community_bridges(graph, args),
        "graph_diff" => handlers::handle_graph_diff(graph, args),
        "pagerank" => handlers::handle_pagerank(graph, args),
        "detect_cycles" => handlers::handle_detect_cycles(graph, args),
        "smart_summary" => handlers::handle_smart_summary(graph, args),
        "find_similar" => handlers::handle_find_similar(graph, args),
        _ => handlers::tool_result_error(&format!("Unknown tool: {tool_name}")),
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
            debug!("Client initialized");
            None
        }
        "tools/list" => {
            debug!("tools/list");
            Some(jsonrpc_response(
                id,
                json!({
                    "tools": tools::tool_definitions()
                }),
            ))
        }
        "tools/call" => Some(dispatch_tools_call(graph, request)),
        "ping" => Some(jsonrpc_response(id, json!({}))),
        _ => {
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

/// Start the MCP server, reading JSON-RPC requests from stdin and writing
/// responses to stdout. Logs go to stderr so they don't interfere with the
/// protocol.
pub fn run_mcp_server(graph_path: &Path) -> Result<(), ServeError> {
    let graph = crate::load_graph(graph_path)?;
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
                if let Ok(json) = serde_json::to_string(&err) {
                    let _ = writeln!(stdout_lock, "{}", json);
                }
                let _ = stdout_lock.flush();
                continue;
            }
        };

        if let Some(response) = dispatch(&graph, &request) {
            let out = match serde_json::to_string(&response) {
                Ok(s) => s,
                Err(e) => {
                    error!("response serialization failed: {e}");
                    continue;
                }
            };
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

#[cfg(test)]
mod tests {
    use super::*;
    use graphify_core::confidence::Confidence;
    use graphify_core::model::{GraphEdge, GraphNode, NodeType};
    use std::collections::HashMap;

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
        assert_eq!(tools.len(), 15);

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
        assert!(text.contains("total_neighbors"));
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

    #[test]
    fn test_find_all_paths() {
        let g = test_graph();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 20,
            "params": {"name": "find_all_paths", "arguments": {
                "source": "auth", "target": "db", "max_length": 4
            }}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(
            parsed["path_count"].as_u64().unwrap() >= 2,
            "should find multiple paths"
        );
    }

    #[test]
    fn test_find_all_paths_no_path() {
        let mut g = KnowledgeGraph::new();
        g.add_node(make_node("x", "X", None)).unwrap();
        g.add_node(make_node("y", "Y", None)).unwrap();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 21,
            "params": {"name": "find_all_paths", "arguments": {
                "source": "x", "target": "y"
            }}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["path_count"].as_u64().unwrap(), 0);
    }

    #[test]
    fn test_weighted_path() {
        let g = test_graph();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 22,
            "params": {"name": "weighted_path", "arguments": {
                "source": "auth", "target": "cache"
            }}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(parsed["path_length"].as_u64().unwrap() >= 1);
        assert!(parsed["total_cost"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn test_weighted_path_not_found() {
        let mut g = KnowledgeGraph::new();
        g.add_node(make_node("x", "X", None)).unwrap();
        g.add_node(make_node("y", "Y", None)).unwrap();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 23,
            "params": {"name": "weighted_path", "arguments": {
                "source": "x", "target": "y"
            }}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("No path found"));
    }

    #[test]
    fn test_community_bridges() {
        let g = test_graph();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 24,
            "params": {"name": "community_bridges", "arguments": {"top_n": 5}}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(parsed["bridges"].as_array().is_some());
    }

    #[test]
    fn test_graph_diff_missing_file() {
        let g = test_graph();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 25,
            "params": {"name": "graph_diff", "arguments": {
                "other_graph": "/nonexistent/graph.json"
            }}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Failed to load graph"));
    }

    #[test]
    fn test_find_all_paths_missing_source() {
        let g = test_graph();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 26,
            "params": {"name": "find_all_paths", "arguments": {
                "source": "nonexistent", "target": "db"
            }}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("not found"));
    }

    #[test]
    fn test_weighted_path_with_min_confidence() {
        let g = test_graph();
        let req = json!({
            "jsonrpc": "2.0", "method": "tools/call", "id": 27,
            "params": {"name": "weighted_path", "arguments": {
                "source": "auth", "target": "db", "min_confidence": 0.5
            }}
        });
        let resp = dispatch(&g, &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(parsed["path_length"].as_u64().unwrap() >= 1);
    }
}
