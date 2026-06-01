//! MCP tool handler implementations.

use std::collections::{HashMap, HashSet, VecDeque};

use graphify_core::graph::KnowledgeGraph;
use serde_json::{Value, json};

use crate::{bfs, graph_stats, score_nodes, subgraph_to_text};

pub(crate) fn tool_result_text(text: &str) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": text
        }]
    })
}

pub(crate) fn tool_result_json<T: serde::Serialize>(value: &T) -> Value {
    let text = serde_json::to_string_pretty(value)
        .unwrap_or_else(|e| format!("{{\"error\": \"serialization failed: {}\"}}", e));
    tool_result_text(&text)
}

pub(crate) fn tool_result_error(text: &str) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": text
        }],
        "isError": true
    })
}

pub(crate) fn handle_query_graph(graph: &KnowledgeGraph, args: &Value) -> Value {
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

pub(crate) fn handle_get_node(graph: &KnowledgeGraph, args: &Value) -> Value {
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
            tool_result_json(&result)
        }
        None => tool_result_error(&format!("Node not found: {node_id}")),
    }
}

pub(crate) fn handle_get_neighbors(graph: &KnowledgeGraph, args: &Value) -> Value {
    let node_id = args["node_id"].as_str().unwrap_or("");
    let depth = args["depth"].as_u64().unwrap_or(1) as usize;
    let limit = args["limit"].as_u64().unwrap_or(50) as usize;
    let offset = args["offset"].as_u64().unwrap_or(0) as usize;

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
            let edge_count = edges
                .iter()
                .filter(|(s, t)| (s == node_id && t == nid) || (s == nid && t == node_id))
                .count();

            neighbor_info.push(json!({
                "id": node.id,
                "label": node.label,
                "node_type": node.node_type,
                "source_file": node.source_file,
                "community": node.community,
                "edge_count": edge_count,
            }));
        }
    }

    let total = neighbor_info.len();
    let paginated: Vec<_> = neighbor_info.into_iter().skip(offset).take(limit).collect();

    let result = json!({
        "node_id": node_id,
        "depth": depth,
        "total_neighbors": total,
        "offset": offset,
        "limit": limit,
        "returned": paginated.len(),
        "neighbors": paginated,
    });

    tool_result_json(&result)
}

pub(crate) fn handle_get_community(graph: &KnowledgeGraph, args: &Value) -> Value {
    let community_id = match args["community_id"].as_u64() {
        Some(id) => id as usize,
        None => return tool_result_error("Missing required parameter: community_id"),
    };
    let limit = args["limit"].as_u64().unwrap_or(50) as usize;
    let offset = args["offset"].as_u64().unwrap_or(0) as usize;

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

    members.sort_by(|a, b| {
        let da = a["degree"].as_u64().unwrap_or(0);
        let db = b["degree"].as_u64().unwrap_or(0);
        db.cmp(&da)
    });

    let total = members.len();
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

pub(crate) fn handle_god_nodes(graph: &KnowledgeGraph, args: &Value) -> Value {
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

    tool_result_json(&output)
}

pub(crate) fn handle_graph_stats(graph: &KnowledgeGraph) -> Value {
    let stats = graph_stats(graph);
    tool_result_json(&stats)
}

pub(crate) fn handle_shortest_path(graph: &KnowledgeGraph, args: &Value) -> Value {
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
        return tool_result_json(&result);
    }

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

    tool_result_json(&result)
}

pub(crate) fn handle_find_all_paths(graph: &KnowledgeGraph, args: &Value) -> Value {
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

    tool_result_json(&result)
}

pub(crate) fn handle_weighted_path(graph: &KnowledgeGraph, args: &Value) -> Value {
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
            tool_result_json(&result)
        }
        None => tool_result_text(&format!(
            "No path found between {source} and {target} with min_confidence {min_confidence}"
        )),
    }
}

pub(crate) fn handle_community_bridges(graph: &KnowledgeGraph, args: &Value) -> Value {
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

    tool_result_json(&output)
}

pub(crate) fn handle_graph_diff(graph: &KnowledgeGraph, args: &Value) -> Value {
    let other_path = args["other_graph"].as_str().unwrap_or("");
    if other_path.is_empty() {
        return tool_result_error("Missing required parameter: other_graph");
    }

    let path = std::path::Path::new(other_path);
    if let Err(e) = graphify_security::validate_graph_path(other_path) {
        return tool_result_error(&format!("Invalid path: {e}"));
    }

    let other_graph = match crate::load_graph(path) {
        Ok(g) => g,
        Err(e) => return tool_result_error(&format!("Failed to load graph: {e}")),
    };

    let diff = graphify_analyze::graph_diff(graph, &other_graph);
    tool_result_json(&diff)
}

pub(crate) fn handle_pagerank(graph: &KnowledgeGraph, args: &Value) -> Value {
    let top_n = args["top_n"].as_u64().unwrap_or(10) as usize;
    let results = graphify_analyze::pagerank(graph, top_n, 0.85, 20);
    tool_result_json(&results)
}

pub(crate) fn handle_detect_cycles(graph: &KnowledgeGraph, args: &Value) -> Value {
    let max_cycles = args["max_cycles"].as_u64().unwrap_or(10) as usize;
    let cycles = graphify_analyze::detect_cycles(graph, max_cycles);
    if cycles.is_empty() {
        tool_result_text("No dependency cycles detected.")
    } else {
        tool_result_json(&cycles)
    }
}

pub(crate) fn handle_smart_summary(graph: &KnowledgeGraph, args: &Value) -> Value {
    let level_str = args["level"].as_str().unwrap_or("community");
    let budget = args["budget"].as_u64().unwrap_or(2000) as usize;

    let level = match level_str {
        "detailed" => crate::SummaryLevel::Detailed,
        "architecture" => crate::SummaryLevel::Architecture,
        _ => crate::SummaryLevel::Community,
    };

    let mut communities: HashMap<usize, Vec<String>> = HashMap::new();
    for node in graph.nodes() {
        let cid = node.community.unwrap_or(0);
        communities.entry(cid).or_default().push(node.id.clone());
    }

    let summary = crate::smart_summary(graph, &communities, level, budget);
    tool_result_text(&summary)
}

pub(crate) fn handle_find_similar(graph: &KnowledgeGraph, args: &Value) -> Value {
    let top_n = args["top_n"].as_u64().unwrap_or(10) as usize;
    let embeddings = graphify_analyze::embedding::compute_embeddings(graph, 64, 10, 40);
    let pairs = graphify_analyze::embedding::find_similar(graph, &embeddings, top_n);
    if pairs.is_empty() {
        tool_result_text("No structurally similar node pairs found.")
    } else {
        tool_result_json(&pairs)
    }
}
