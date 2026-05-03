//! Lightweight local HTTP transport for graphify-rs MCP.
//!
//! This intentionally keeps the default stdio transport untouched while adding
//! a local-only HTTP surface that is easy for Codex helpers and short-lived
//! CLI clients to reuse. It is deliberately small: one JSON-RPC request per
//! HTTP request, plus a few `graphifyq` convenience endpoints.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use graphify_core::graph::KnowledgeGraph;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info};

use crate::mcp::handle_jsonrpc;
use crate::{ServeError, graph_stats, load_graph};

#[derive(Debug, Clone)]
pub struct HttpServerConfig {
    pub bind: String,
    pub mcp_path: String,
    pub registry_path: Option<PathBuf>,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:0".to_string(),
            mcp_path: "/mcp".to_string(),
            registry_path: None,
        }
    }
}

pub async fn start_http_server(
    graph_path: &Path,
    config: HttpServerConfig,
) -> Result<(), ServeError> {
    let graph = Arc::new(load_graph(graph_path)?);
    let listener = TcpListener::bind(&config.bind).await?;
    let addr = listener.local_addr()?;
    let http_url = format!("http://{addr}");
    let mcp_path = normalize_path(&config.mcp_path);

    if let Some(registry_path) = &config.registry_path {
        write_registry(registry_path, graph_path, &http_url, &mcp_path)?;
    }

    let stats = graph_stats(&graph);
    let node_count = stats
        .get("node_count")
        .map_or_else(|| "null".to_string(), ToString::to_string);
    let edge_count = stats
        .get("edge_count")
        .map_or_else(|| "null".to_string(), ToString::to_string);
    info!(
        "HTTP MCP server listening on {http_url}{mcp_path}: {} nodes, {} edges",
        node_count, edge_count,
    );

    loop {
        let (stream, _) = listener.accept().await?;
        let graph = Arc::clone(&graph);
        let mcp_path = mcp_path.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_connection(stream, graph, &mcp_path).await {
                debug!("HTTP connection failed: {err}");
            }
        });
    }
}

fn write_registry(
    registry_path: &Path,
    graph_path: &Path,
    http_url: &str,
    mcp_path: &str,
) -> Result<(), ServeError> {
    if let Some(parent) = registry_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let graph_path = graph_path
        .canonicalize()
        .unwrap_or_else(|_| graph_path.to_path_buf());

    let payload = json!({
        "root": cwd,
        "pid": std::process::id(),
        "http_url": http_url,
        "mcp_url": format!("{http_url}{mcp_path}"),
        "graphifyq_url": format!("{http_url}/graphifyq"),
        "graph_path": graph_path,
        "started_at_ms": now_ms,
    });

    std::fs::write(registry_path, serde_json::to_vec_pretty(&payload)?)?;
    Ok(())
}

async fn handle_connection(
    mut stream: TcpStream,
    graph: Arc<KnowledgeGraph>,
    mcp_path: &str,
) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(8192);

    let header_end = loop {
        let mut chunk = [0_u8; 2048];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_header_end(&buf) {
            break pos;
        }
        if buf.len() > 1024 * 1024 {
            write_response(
                &mut stream,
                413,
                "Payload Too Large",
                "application/json",
                br#"{"error":"request headers too large"}"#,
            )
            .await?;
            return Ok(());
        }
    };

    let header = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = header.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default().to_string();
    let path = request_parts.next().unwrap_or_default().to_string();
    let content_length = content_length(&header);

    let body_start = header_end + 4;
    while buf.len() < body_start + content_length {
        let mut chunk = [0_u8; 8192];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body = &buf[body_start..buf.len().min(body_start + content_length)];

    match (method.as_str(), path.as_str()) {
        ("GET", "/health") => {
            let stats = graph_stats(&graph);
            let payload = json!({
                "ok": true,
                "server": "graphify-rs",
                "version": env!("CARGO_PKG_VERSION"),
                "stats": stats,
            });
            write_json(&mut stream, 200, &payload).await
        }
        ("GET", "/graphifyq/stats") => {
            let payload = json!(graph_stats(&graph));
            write_json(&mut stream, 200, &payload).await
        }
        ("POST", p) if p == mcp_path => handle_mcp_post(&mut stream, &graph, body).await,
        ("POST", "/graphifyq/query") => handle_query_post(&mut stream, &graph, body).await,
        ("POST", "/graphifyq/tool") => handle_tool_post(&mut stream, &graph, body).await,
        ("OPTIONS", _) => write_response(&mut stream, 204, "No Content", "text/plain", b"").await,
        _ => {
            let payload = json!({
                "error": "not_found",
                "message": format!("No route for {method} {path}"),
            });
            write_json(&mut stream, 404, &payload).await
        }
    }
}

async fn handle_mcp_post(
    stream: &mut TcpStream,
    graph: &KnowledgeGraph,
    body: &[u8],
) -> std::io::Result<()> {
    let request: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(err) => {
            let payload = json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {"code": -32700, "message": format!("Parse error: {err}")},
            });
            return write_json(stream, 400, &payload).await;
        }
    };

    if let Some(items) = request.as_array() {
        let responses: Vec<Value> = items
            .iter()
            .filter_map(|item| handle_jsonrpc(graph, item))
            .collect();
        if responses.is_empty() {
            write_response(stream, 202, "Accepted", "text/plain", b"").await
        } else {
            write_json(stream, 200, &Value::Array(responses)).await
        }
    } else if let Some(response) = handle_jsonrpc(graph, &request) {
        write_json(stream, 200, &response).await
    } else {
        write_response(stream, 202, "Accepted", "text/plain", b"").await
    }
}

async fn handle_query_post(
    stream: &mut TcpStream,
    graph: &KnowledgeGraph,
    body: &[u8],
) -> std::io::Result<()> {
    let request: Value = serde_json::from_slice(body).unwrap_or_else(|_| json!({}));
    let question = request["question"].as_str().unwrap_or_default();
    if question.is_empty() {
        return write_json(
            stream,
            400,
            &json!({"error": "missing required field: question"}),
        )
        .await;
    }
    let arguments = json!({
        "question": question,
        "budget": request["budget"].as_u64().unwrap_or(2000),
    });
    call_tool(stream, graph, "query_graph", arguments).await
}

async fn handle_tool_post(
    stream: &mut TcpStream,
    graph: &KnowledgeGraph,
    body: &[u8],
) -> std::io::Result<()> {
    let request: Value = serde_json::from_slice(body).unwrap_or_else(|_| json!({}));
    let name = request["name"].as_str().unwrap_or_default();
    if name.is_empty() {
        return write_json(
            stream,
            400,
            &json!({"error": "missing required field: name"}),
        )
        .await;
    }
    let arguments = request
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    call_tool(stream, graph, name, arguments).await
}

async fn call_tool(
    stream: &mut TcpStream,
    graph: &KnowledgeGraph,
    name: &str,
    arguments: Value,
) -> std::io::Result<()> {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": arguments,
        }
    });
    let response = handle_jsonrpc(graph, &request).unwrap_or_else(|| json!({}));
    write_json(stream, 200, &response).await
}

async fn write_json(stream: &mut TcpStream, status: u16, value: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "OK",
    };
    write_response(stream, status, reason, "application/json", &body).await
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Access-Control-Allow-Origin: http://localhost\r\n\
         Access-Control-Allow-Headers: content-type, mcp-session-id\r\n\
         Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn content_length(header: &str) -> usize {
    header
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0)
}

fn normalize_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use graphify_core::confidence::Confidence;
    use graphify_core::model::{GraphEdge, GraphNode, NodeType};
    use serde::Deserialize;
    use tempfile::tempdir;
    use tokio::time::{Duration, sleep};

    fn make_graph() -> KnowledgeGraph {
        let mut graph = KnowledgeGraph::new();
        graph
            .add_node(GraphNode {
                id: "auth".to_string(),
                label: "AuthService".to_string(),
                source_file: "src/auth.rs".to_string(),
                source_location: None,
                node_type: NodeType::Class,
                community: Some(0),
                extra: HashMap::new(),
            })
            .unwrap();
        graph
            .add_node(GraphNode {
                id: "db".to_string(),
                label: "Database".to_string(),
                source_file: "src/db.rs".to_string(),
                source_location: None,
                node_type: NodeType::Class,
                community: Some(0),
                extra: HashMap::new(),
            })
            .unwrap();
        graph
            .add_edge(GraphEdge {
                source: "auth".to_string(),
                target: "db".to_string(),
                relation: "uses".to_string(),
                confidence: Confidence::Extracted,
                confidence_score: 1.0,
                source_file: "src/auth.rs".to_string(),
                source_location: None,
                weight: 1.0,
                extra: HashMap::new(),
            })
            .unwrap();
        graph
    }

    #[test]
    fn normalize_path_adds_leading_slash() {
        assert_eq!(normalize_path("mcp"), "/mcp");
        assert_eq!(normalize_path("/mcp"), "/mcp");
    }

    #[test]
    fn content_length_is_case_insensitive() {
        let header = "POST /mcp HTTP/1.1\r\ncontent-length: 42\r\n\r\n";
        assert_eq!(content_length(header), 42);
    }

    #[tokio::test]
    async fn http_server_serves_health_mcp_and_graphifyq_endpoints() {
        let dir = tempdir().unwrap();
        let graph_path = dir.path().join("graph.json");
        let registry_path = dir.path().join("server.json");
        let graph = make_graph();
        std::fs::write(
            &graph_path,
            serde_json::to_vec(&graph.to_node_link_json()).unwrap(),
        )
        .unwrap();

        let server_graph_path = graph_path.clone();
        let server_registry_path = registry_path.clone();
        let handle = tokio::spawn(async move {
            start_http_server(
                &server_graph_path,
                HttpServerConfig {
                    bind: "127.0.0.1:0".to_string(),
                    mcp_path: "mcp".to_string(),
                    registry_path: Some(server_registry_path),
                },
            )
            .await
        });

        let registry = wait_for_registry(&registry_path).await;
        let client = reqwest::Client::new();

        let health: Value = client
            .get(format!("{}/health", registry.http_url))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(health["ok"], true);
        assert_eq!(health["stats"]["node_count"], 2);

        let initialize: Value = client
            .post(&registry.mcp_url)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "0"}
                }
            }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(initialize["result"]["serverInfo"]["name"], "graphify-rs");

        let query: Value = client
            .post(format!("{}/graphifyq/query", registry.http_url))
            .json(&json!({"question": "auth database", "budget": 500}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let text = query["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("AuthService"));
        assert!(text.contains("Database"));

        let stats: Value = client
            .get(format!("{}/graphifyq/stats", registry.http_url))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(stats["edge_count"], 1);

        handle.abort();
    }

    #[derive(Deserialize)]
    struct TestRegistry {
        http_url: String,
        mcp_url: String,
    }

    async fn wait_for_registry(path: &Path) -> TestRegistry {
        for _ in 0..50 {
            if let Ok(content) = std::fs::read_to_string(path)
                && let Ok(registry) = serde_json::from_str(&content)
            {
                return registry;
            }
            sleep(Duration::from_millis(20)).await;
        }
        panic!("registry was not written");
    }
}
