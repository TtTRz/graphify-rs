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
        },
        {
            "name": "find_all_paths",
            "description": "Find all simple paths between two nodes up to a maximum length.",
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
                    },
                    "max_length": {
                        "type": "number",
                        "description": "Maximum path length in edges (default: 4)",
                        "default": 4
                    }
                },
                "required": ["source", "target"]
            }
        },
        {
            "name": "weighted_path",
            "description": "Find the shortest weighted path between two nodes using Dijkstra's algorithm. Higher edge weights mean shorter distance.",
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
                    },
                    "min_confidence": {
                        "type": "number",
                        "description": "Minimum confidence score for edges to consider (default: 0.0)",
                        "default": 0.0
                    }
                },
                "required": ["source", "target"]
            }
        },
        {
            "name": "community_bridges",
            "description": "Find nodes that bridge multiple communities. These nodes connect different parts of the codebase.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "top_n": {
                        "type": "number",
                        "description": "Number of top bridge nodes to return (default: 10)",
                        "default": 10
                    }
                }
            }
        },
        {
            "name": "graph_diff",
            "description": "Compare the current graph with another graph file. Shows added and removed nodes and edges.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "other_graph": {
                        "type": "string",
                        "description": "Path to the other graph.json file to compare against"
                    }
                },
                "required": ["other_graph"]
            }
        },
        {
            "name": "pagerank",
            "description": "Compute PageRank importance scores. Unlike degree-based ranking, PageRank identifies nodes that are important due to being connected to other important nodes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "top_n": {
                        "type": "number",
                        "description": "Number of top nodes to return (default: 10)"
                    }
                }
            }
        },
        {
            "name": "detect_cycles",
            "description": "Detect dependency cycles in the graph using Tarjan's algorithm. Finds circular dependencies (A imports B imports C imports A) that indicate architectural debt.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "max_cycles": {
                        "type": "number",
                        "description": "Maximum number of cycles to return (default: 10)"
                    }
                }
            }
        },
        {
            "name": "smart_summary",
            "description": "Generate a multi-level graph summary. Level 'detailed' shows all nodes, 'community' shows one representative per community, 'architecture' groups by directory.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "level": {
                        "type": "string",
                        "description": "Summary level: detailed, community, or architecture (default: community)",
                        "enum": ["detailed", "community", "architecture"]
                    },
                    "budget": {
                        "type": "number",
                        "description": "Token budget for summary (default: 2000)"
                    }
                }
            }
        },
        {
            "name": "find_similar",
            "description": "Find structurally similar node pairs using graph embeddings. Identifies nodes with similar connectivity patterns that may be redundant or candidates for refactoring.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "top_n": {
                        "type": "number",
                        "description": "Number of similar pairs to return (default: 10)"
                    }
                }
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

fn handle_find_all_paths(graph: &KnowledgeGraph, args: &Value) -> Value {
    let source = args["source"].as_str().unwrap_or("");
    let target = args["target"].as_str().unwrap_or("");
    let max_length = args["max_length"].as_u64().unwrap_or(4) as usize;

    if source.is_empty() || target.is_empty() {
        return tool_result_error("Missing required parameters: source and target");
    }
    if graph.get_node(source).is_none() {
        return tool_result_error(&format!("Source node not found: {source}"));
    }
    if graph.get_node(target).is_none() {
        return tool_result_error(&format!("Target node not found: {target}"));
    }

    let paths = crate::all_simple_paths(graph, source, target, max_length);

    let paths_json: Vec<Value> = paths
        .iter()
        .map(|path| {
            let nodes: Vec<Value> = path
                .iter()
                .map(|id| {
                    let label = graph.get_node(id).map(|n| n.label.as_str()).unwrap_or(id);
                    json!({"id": id, "label": label})
                })
                .collect();
            json!({
                "length": path.len() - 1,
                "nodes": nodes
            })
        })
        .collect();

    let result = json!({
        "source": source,
        "target": target,
        "max_length": max_length,
        "path_count": paths_json.len(),
        "paths": paths_json,
    });

    tool_result_text(&serde_json::to_string_pretty(&result).unwrap_or_default())
}

fn handle_weighted_path(graph: &KnowledgeGraph, args: &Value) -> Value {
    let source = args["source"].as_str().unwrap_or("");
    let target = args["target"].as_str().unwrap_or("");
    let min_confidence = args["min_confidence"].as_f64().unwrap_or(0.0);

    if source.is_empty() || target.is_empty() {
        return tool_result_error("Missing required parameters: source and target");
    }
    if graph.get_node(source).is_none() {
        return tool_result_error(&format!("Source node not found: {source}"));
    }
    if graph.get_node(target).is_none() {
        return tool_result_error(&format!("Target node not found: {target}"));
    }

    match crate::dijkstra_path(graph, source, target, min_confidence) {
        Some((path, total_cost, edge_details)) => {
            let path_nodes: Vec<Value> = path
                .iter()
                .map(|id| {
                    let label = graph.get_node(id).map(|n| n.label.as_str()).unwrap_or(id);
                    json!({"id": id, "label": label})
                })
                .collect();

            let edges: Vec<Value> = edge_details
                .iter()
                .map(|(from, to, cost, relation)| {
                    json!({
                        "from": from,
                        "to": to,
                        "cost": cost,
                        "relation": relation
                    })
                })
                .collect();

            let result = json!({
                "source": source,
                "target": target,
                "min_confidence": min_confidence,
                "total_cost": total_cost,
                "path_length": path.len() - 1,
                "path": path_nodes,
                "edges": edges,
            });
            tool_result_text(&serde_json::to_string_pretty(&result).unwrap_or_default())
        }
        None => tool_result_text(&format!(
            "No path found between {source} and {target} with min_confidence {min_confidence}"
        )),
    }
}

fn handle_community_bridges(graph: &KnowledgeGraph, args: &Value) -> Value {
    // Build communities from node.community field
    let mut communities: HashMap<usize, Vec<String>> = HashMap::new();
    for node_id in graph.node_ids() {
        if let Some(node) = graph.get_node(&node_id)
            && let Some(cid) = node.community
        {
            communities.entry(cid).or_default().push(node_id);
        }
    }

    let top_n = args["top_n"].as_u64().unwrap_or(10) as usize;
    let bridges = graphify_analyze::community_bridges(graph, &communities, top_n);

    let result: Vec<Value> = bridges
        .iter()
        .map(|b| {
            json!({
                "id": b.id,
                "label": b.label,
                "total_edges": b.total_edges,
                "cross_community_edges": b.cross_community_edges,
                "bridge_ratio": format!("{:.2}", b.bridge_ratio),
                "communities_touched": b.communities_touched,
            })
        })
        .collect();

    let output = json!({
        "top_n": top_n,
        "bridge_count": result.len(),
        "bridges": result,
    });

    tool_result_text(&serde_json::to_string_pretty(&output).unwrap_or_default())
}

fn handle_graph_diff(graph: &KnowledgeGraph, args: &Value) -> Value {
    let other_path = args["other_graph"].as_str().unwrap_or("");
    if other_path.is_empty() {
        return tool_result_error("Missing required parameter: other_graph");
    }

    let other_graph = match crate::load_graph(std::path::Path::new(other_path)) {
        Ok(g) => g,
        Err(e) => return tool_result_error(&format!("Failed to load graph: {e}")),
    };

    let diff = graphify_analyze::graph_diff(graph, &other_graph);
    tool_result_text(&serde_json::to_string_pretty(&diff).unwrap_or_default())
}

fn handle_pagerank(graph: &KnowledgeGraph, args: &Value) -> Value {
    let top_n = args["top_n"].as_u64().unwrap_or(10) as usize;
    let results = graphify_analyze::pagerank(graph, top_n, 0.85, 20);
    tool_result_text(&serde_json::to_string_pretty(&results).unwrap_or_default())
}

fn handle_detect_cycles(graph: &KnowledgeGraph, args: &Value) -> Value {
    let max_cycles = args["max_cycles"].as_u64().unwrap_or(10) as usize;
    let cycles = graphify_analyze::detect_cycles(graph, max_cycles);
    if cycles.is_empty() {
        tool_result_text("No dependency cycles detected.")
    } else {
        tool_result_text(&serde_json::to_string_pretty(&cycles).unwrap_or_default())
    }
}

fn handle_smart_summary(graph: &KnowledgeGraph, args: &Value) -> Value {
    let level_str = args["level"].as_str().unwrap_or("community");
    let budget = args["budget"].as_u64().unwrap_or(2000) as usize;

    let level = match level_str {
        "detailed" => crate::SummaryLevel::Detailed,
        "architecture" => crate::SummaryLevel::Architecture,
        _ => crate::SummaryLevel::Community,
    };

    // Build communities map from graph node.community field
    let mut communities: HashMap<usize, Vec<String>> = HashMap::new();
    for node in graph.nodes() {
        let cid = node.community.unwrap_or(0);
        communities.entry(cid).or_default().push(node.id.clone());
    }

    let summary = crate::smart_summary(graph, &communities, level, budget);
    tool_result_text(&summary)
}

fn handle_find_similar(graph: &KnowledgeGraph, args: &Value) -> Value {
    let top_n = args["top_n"].as_u64().unwrap_or(10) as usize;
    let embeddings = graphify_analyze::embedding::compute_embeddings(graph, 64, 10, 40);
    let pairs = graphify_analyze::embedding::find_similar(graph, &embeddings, top_n);
    if pairs.is_empty() {
        tool_result_text("No structurally similar node pairs found.")
    } else {
        tool_result_text(&serde_json::to_string_pretty(&pairs).unwrap_or_default())
    }
}

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
        "find_all_paths" => handle_find_all_paths(graph, args),
        "weighted_path" => handle_weighted_path(graph, args),
        "community_bridges" => handle_community_bridges(graph, args),
        "graph_diff" => handle_graph_diff(graph, args),
        "pagerank" => handle_pagerank(graph, args),
        "detect_cycles" => handle_detect_cycles(graph, args),
        "smart_summary" => handle_smart_summary(graph, args),
        "find_similar" => handle_find_similar(graph, args),
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

    // -- New tool tests --

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
        // test_graph has 2 communities, bridge nodes should exist
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
