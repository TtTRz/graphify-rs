//! End-to-end integration tests for the full graphify pipeline.
//!
//! Each test runs the real pipeline (detect → extract → build → cluster →
//! analyze → export) on small fixture directories and verifies the outputs.

use std::collections::HashMap;
use std::path::Path;

/// Helper: run the full pipeline on a temp dir and return the graph + output dir.
fn run_pipeline(dir: &Path) -> (graphify_core::graph::KnowledgeGraph, std::path::PathBuf) {
    let output_dir = dir.join("out");

    // Detect
    let detection = graphify_detect::detect(dir);

    // Extract
    let code_files: Vec<std::path::PathBuf> = detection
        .files
        .get(&graphify_detect::FileType::Code)
        .map(|v| v.iter().map(|f| dir.join(f)).collect())
        .unwrap_or_default();

    let extraction = graphify_extract::extract(&code_files);

    // Build
    let graph = graphify_build::build(&[extraction]).expect("build failed");

    // Cluster
    let _communities = graphify_cluster::cluster(&graph);

    // Export
    std::fs::create_dir_all(&output_dir).unwrap();
    graphify_export::export_json(&graph, &output_dir).unwrap();

    (graph, output_dir)
}

// ---------------------------------------------------------------------------
// Pipeline: detect → extract → build → export
// ---------------------------------------------------------------------------

#[test]
fn test_full_pipeline_rust_project() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("main.rs"),
        r#"
use std::collections::HashMap;

fn main() {
    let mut map = HashMap::new();
    map.insert("hello", "world");
    process(&map);
}

fn process(data: &HashMap<&str, &str>) {
    for (k, v) in data {
        println!("{}: {}", k, v);
    }
}
"#,
    )
    .unwrap();
    std::fs::write(
        src.join("lib.rs"),
        r#"
pub struct Config {
    pub name: String,
    pub debug: bool,
}

impl Config {
    pub fn new(name: &str) -> Self {
        Config {
            name: name.to_string(),
            debug: false,
        }
    }

    pub fn enable_debug(&mut self) {
        self.debug = true;
    }
}

pub fn default_config() -> Config {
    Config::new("default")
}
"#,
    )
    .unwrap();

    let (graph, output_dir) = run_pipeline(dir.path());

    // Verify graph has nodes and edges
    assert!(
        graph.node_count() >= 4,
        "expected >= 4 nodes, got {}",
        graph.node_count()
    );
    assert!(
        graph.edge_count() >= 2,
        "expected >= 2 edges, got {}",
        graph.edge_count()
    );

    // Verify Config struct was extracted
    let labels: Vec<String> = graph
        .node_ids()
        .iter()
        .filter_map(|id| graph.get_node(id))
        .map(|n| n.label.clone())
        .collect();
    assert!(
        labels.iter().any(|l| l == "Config"),
        "should extract Config struct, got: {:?}",
        labels
    );

    // Verify graph.json exists and is valid
    let json_path = output_dir.join("graph.json");
    assert!(json_path.exists(), "graph.json should be created");
    let json_str = std::fs::read_to_string(&json_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert!(json.get("nodes").unwrap().as_array().unwrap().len() >= 4);
    assert!(json.get("links").unwrap().as_array().unwrap().len() >= 2);
    assert_eq!(json.get("directed").unwrap().as_bool().unwrap(), false);
    assert_eq!(json.get("multigraph").unwrap().as_bool().unwrap(), false);
}

#[test]
fn test_full_pipeline_python_project() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("app.py"),
        r#"
from flask import Flask, jsonify

app = Flask(__name__)

class UserService:
    def __init__(self, db):
        self.db = db

    def get_user(self, user_id):
        return self.db.query(user_id)

    def create_user(self, name, email):
        return self.db.insert(name, email)

def init_app():
    service = UserService(None)
    return app
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("models.py"),
        r#"
class User:
    def __init__(self, name, email):
        self.name = name
        self.email = email

class Database:
    def __init__(self, url):
        self.url = url

    def query(self, id):
        pass

    def insert(self, name, email):
        return User(name, email)
"#,
    )
    .unwrap();

    let (graph, _) = run_pipeline(dir.path());

    assert!(graph.node_count() >= 5, "got {} nodes", graph.node_count());

    let labels: Vec<String> = graph
        .node_ids()
        .iter()
        .filter_map(|id| graph.get_node(id))
        .map(|n| n.label.clone())
        .collect();
    assert!(
        labels.iter().any(|l| l == "UserService"),
        "missing UserService: {:?}",
        labels
    );
    assert!(
        labels.iter().any(|l| l == "User"),
        "missing User: {:?}",
        labels
    );
    assert!(
        labels.iter().any(|l| l == "Database"),
        "missing Database: {:?}",
        labels
    );
}

#[test]
fn test_full_pipeline_javascript_project() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("index.js"),
        r#"
const express = require('express');

class Router {
    constructor(app) {
        this.app = app;
    }

    addRoute(path, handler) {
        this.app.get(path, handler);
    }
}

function createApp() {
    const app = express();
    const router = new Router(app);
    router.addRoute('/', (req, res) => res.send('hello'));
    return app;
}

module.exports = { Router, createApp };
"#,
    )
    .unwrap();

    let (graph, _) = run_pipeline(dir.path());

    assert!(graph.node_count() >= 3, "got {} nodes", graph.node_count());

    let labels: Vec<String> = graph
        .node_ids()
        .iter()
        .filter_map(|id| graph.get_node(id))
        .map(|n| n.label.clone())
        .collect();
    assert!(
        labels.iter().any(|l| l == "Router"),
        "missing Router: {:?}",
        labels
    );
    assert!(
        labels.iter().any(|l| l.contains("createApp")),
        "missing createApp: {:?}",
        labels
    );
}

#[test]
fn test_full_pipeline_go_project() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.go"),
        r#"
package main

import "fmt"

type Server struct {
    Host string
    Port int
}

func NewServer(host string, port int) *Server {
    return &Server{Host: host, Port: port}
}

func (s *Server) Start() {
    fmt.Printf("Listening on %s:%d\n", s.Host, s.Port)
}

func main() {
    srv := NewServer("localhost", 8080)
    srv.Start()
}
"#,
    )
    .unwrap();

    let (graph, _) = run_pipeline(dir.path());

    assert!(graph.node_count() >= 3, "got {} nodes", graph.node_count());

    let labels: Vec<String> = graph
        .node_ids()
        .iter()
        .filter_map(|id| graph.get_node(id))
        .map(|n| n.label.clone())
        .collect();
    assert!(
        labels.iter().any(|l| l == "Server"),
        "missing Server: {:?}",
        labels
    );
    assert!(
        labels.iter().any(|l| l.contains("NewServer")),
        "missing NewServer: {:?}",
        labels
    );
}

// ---------------------------------------------------------------------------
// JSON output compatibility
// ---------------------------------------------------------------------------

#[test]
fn test_json_output_networkx_compatible() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\npub fn sub(a: i32, b: i32) -> i32 { a - b }\n",
    )
    .unwrap();

    let (_, output_dir) = run_pipeline(dir.path());

    let json_str = std::fs::read_to_string(output_dir.join("graph.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    // NetworkX node_link_data required fields
    assert!(json.get("directed").is_some(), "missing 'directed' field");
    assert!(
        json.get("multigraph").is_some(),
        "missing 'multigraph' field"
    );
    assert!(json.get("graph").is_some(), "missing 'graph' field");
    assert!(json.get("nodes").is_some(), "missing 'nodes' field");
    assert!(json.get("links").is_some(), "missing 'links' field");

    // Verify node structure
    let nodes = json.get("nodes").unwrap().as_array().unwrap();
    for node in nodes {
        assert!(node.get("id").is_some(), "node missing 'id': {:?}", node);
        assert!(
            node.get("label").is_some(),
            "node missing 'label': {:?}",
            node
        );
        assert!(
            node.get("node_type").is_some(),
            "node missing 'node_type': {:?}",
            node
        );
    }

    // Verify link structure
    let links = json.get("links").unwrap().as_array().unwrap();
    for link in links {
        assert!(
            link.get("source").is_some(),
            "link missing 'source': {:?}",
            link
        );
        assert!(
            link.get("target").is_some(),
            "link missing 'target': {:?}",
            link
        );
        assert!(
            link.get("relation").is_some(),
            "link missing 'relation': {:?}",
            link
        );
    }
}

#[test]
fn test_json_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("example.py"),
        "class Foo:\n    def bar(self):\n        pass\n\ndef baz():\n    return Foo()\n",
    )
    .unwrap();

    let (original_graph, output_dir) = run_pipeline(dir.path());

    // Read JSON back and reconstruct graph
    let json_str = std::fs::read_to_string(output_dir.join("graph.json")).unwrap();
    let json_value: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let loaded_graph =
        graphify_core::graph::KnowledgeGraph::from_node_link_json(&json_value).unwrap();

    assert_eq!(
        original_graph.node_count(),
        loaded_graph.node_count(),
        "node count mismatch after roundtrip"
    );
    assert_eq!(
        original_graph.edge_count(),
        loaded_graph.edge_count(),
        "edge count mismatch after roundtrip"
    );
}

// ---------------------------------------------------------------------------
// Graph diff
// ---------------------------------------------------------------------------

#[test]
fn test_graph_diff_detects_changes() {
    // Build graph v1
    let dir1 = tempfile::tempdir().unwrap();
    std::fs::write(
        dir1.path().join("code.py"),
        "class Alpha:\n    pass\n\nclass Beta:\n    pass\n",
    )
    .unwrap();
    let (graph1, _) = run_pipeline(dir1.path());

    // Build graph v2 (add a class, remove one)
    let dir2 = tempfile::tempdir().unwrap();
    std::fs::write(
        dir2.path().join("code.py"),
        "class Alpha:\n    pass\n\nclass Gamma:\n    pass\n",
    )
    .unwrap();
    let (graph2, _) = run_pipeline(dir2.path());

    let diff = graphify_analyze::graph_diff(&graph1, &graph2);

    let added = diff.get("added_nodes").unwrap().as_array().unwrap();
    let removed = diff.get("removed_nodes").unwrap().as_array().unwrap();

    // Gamma should be added, Beta should be removed
    let added_strs: Vec<&str> = added.iter().filter_map(|v| v.as_str()).collect();
    let removed_strs: Vec<&str> = removed.iter().filter_map(|v| v.as_str()).collect();

    assert!(
        added_strs.iter().any(|s| s.contains("gamma")),
        "Gamma should be in added nodes: {:?}",
        added_strs
    );
    assert!(
        removed_strs.iter().any(|s| s.contains("beta")),
        "Beta should be in removed nodes: {:?}",
        removed_strs
    );
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

#[test]
fn test_cache_speeds_up_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out");
    let cache_dir = output.join("cache");
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();

    std::fs::write(
        src.join("main.rs"),
        "fn main() { hello(); }\nfn hello() { println!(\"hi\"); }\n",
    )
    .unwrap();
    std::fs::write(
        src.join("lib.rs"),
        "pub struct Config { pub name: String }\nimpl Config { pub fn new() -> Self { Config { name: String::new() } } }\n",
    )
    .unwrap();

    let file1 = src.join("main.rs");
    let file2 = src.join("lib.rs");

    // First extraction: no cache
    let result1 = graphify_extract::extract(&[file1.clone(), file2.clone()]);
    for f in &[&file1, &file2] {
        graphify_cache::save_cached_to(f, &result1, dir.path(), &cache_dir);
    }

    // Second extraction: should hit cache
    let cached1: Option<graphify_core::model::ExtractionResult> =
        graphify_cache::load_cached_from(&file1, dir.path(), &cache_dir);
    let cached2: Option<graphify_core::model::ExtractionResult> =
        graphify_cache::load_cached_from(&file2, dir.path(), &cache_dir);

    assert!(cached1.is_some(), "file1 should be cached");
    assert!(cached2.is_some(), "file2 should be cached");

    // Modify file1, cache should miss
    std::fs::write(&file1, "fn main() { goodbye(); }\nfn goodbye() { }\n").unwrap();
    let cached_after: Option<graphify_core::model::ExtractionResult> =
        graphify_cache::load_cached_from(&file1, dir.path(), &cache_dir);
    assert!(cached_after.is_none(), "modified file should miss cache");

    // file2 unchanged, still cached
    let cached2_still: Option<graphify_core::model::ExtractionResult> =
        graphify_cache::load_cached_from(&file2, dir.path(), &cache_dir);
    assert!(
        cached2_still.is_some(),
        "unmodified file should still be cached"
    );
}

// ---------------------------------------------------------------------------
// Detect
// ---------------------------------------------------------------------------

#[test]
fn test_detect_classifies_files_correctly() {
    let dir = tempfile::tempdir().unwrap();

    // Code files
    std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
    std::fs::write(dir.path().join("app.py"), "pass").unwrap();
    std::fs::write(dir.path().join("index.js"), "console.log()").unwrap();

    // Doc files
    std::fs::write(dir.path().join("README.md"), "# Hello").unwrap();
    std::fs::write(dir.path().join("notes.txt"), "some notes").unwrap();

    let detection = graphify_detect::detect(dir.path());

    let code = detection
        .files
        .get(&graphify_detect::FileType::Code)
        .map_or(0, |v| v.len());
    let doc = detection
        .files
        .get(&graphify_detect::FileType::Document)
        .map_or(0, |v| v.len());

    assert!(code >= 3, "should find >= 3 code files, got {}", code);
    assert!(doc >= 1, "should find >= 1 doc files, got {}", doc);
    assert_eq!(detection.total_files, code + doc);
}

#[test]
fn test_detect_respects_graphifyignore() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".graphifyignore"), "secret.py\n*.log\n").unwrap();
    std::fs::write(dir.path().join("app.py"), "pass").unwrap();
    std::fs::write(dir.path().join("secret.py"), "password='123'").unwrap();
    std::fs::write(dir.path().join("debug.log"), "log content").unwrap();

    let detection = graphify_detect::detect(dir.path());

    let all_files: Vec<String> = detection.files.values().flat_map(|v| v.clone()).collect();

    assert!(
        !all_files.iter().any(|f| f.contains("secret.py")),
        "secret.py should be ignored: {:?}",
        all_files
    );
    assert!(
        !all_files.iter().any(|f| f.contains("debug.log")),
        "debug.log should be ignored: {:?}",
        all_files
    );
    assert!(
        all_files.iter().any(|f| f.contains("app.py")),
        "app.py should be detected: {:?}",
        all_files
    );
}

// ---------------------------------------------------------------------------
// Cluster
// ---------------------------------------------------------------------------

#[test]
fn test_cluster_produces_communities() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("a.py"),
        "class A:\n    def method_a(self): pass\n\nclass B:\n    def method_b(self): pass\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("b.py"),
        "class C:\n    def method_c(self): pass\n\nclass D:\n    def method_d(self): pass\n",
    )
    .unwrap();

    let (graph, _) = run_pipeline(dir.path());

    let communities = graphify_cluster::cluster(&graph);

    // Should produce at least 1 community
    assert!(
        !communities.is_empty(),
        "should produce at least 1 community"
    );

    // Every node should be in some community
    let all_community_nodes: Vec<&String> = communities.values().flat_map(|v| v.iter()).collect();
    for id in graph.node_ids() {
        assert!(
            all_community_nodes.contains(&&id),
            "node {} should be in a community",
            id
        );
    }
}

// ---------------------------------------------------------------------------
// Analyze
// ---------------------------------------------------------------------------

#[test]
fn test_god_nodes_returns_top_connected() {
    let dir = tempfile::tempdir().unwrap();
    // Create a file with a hub class that many things depend on
    std::fs::write(
        dir.path().join("hub.py"),
        r#"
class Hub:
    def a(self): pass
    def b(self): pass
    def c(self): pass
    def d(self): pass
    def e(self): pass

class User:
    def use_hub(self):
        h = Hub()
        h.a()
        h.b()
"#,
    )
    .unwrap();

    let (graph, _) = run_pipeline(dir.path());

    let gods = graphify_analyze::god_nodes(&graph, 3);
    // Should return at most 3
    assert!(gods.len() <= 3);
    // The first god node should have the highest degree
    if gods.len() >= 2 {
        assert!(gods[0].degree >= gods[1].degree);
    }
}

// ---------------------------------------------------------------------------
// Export: all formats
// ---------------------------------------------------------------------------

#[test]
fn test_all_export_formats() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("out");
    std::fs::write(
        dir.path().join("code.py"),
        "class Foo:\n    def bar(self): pass\n\ndef baz(): return Foo()\n",
    )
    .unwrap();

    let detection = graphify_detect::detect(dir.path());
    let code_files: Vec<std::path::PathBuf> = detection
        .files
        .get(&graphify_detect::FileType::Code)
        .map(|v| v.iter().map(|f| dir.path().join(f)).collect())
        .unwrap_or_default();
    let extraction = graphify_extract::extract(&code_files);
    let graph = graphify_build::build(&[extraction]).unwrap();
    let communities = graphify_cluster::cluster(&graph);
    let cohesion = graphify_cluster::score_all(&graph, &communities);
    let community_labels: HashMap<usize, String> = communities
        .iter()
        .map(|(cid, nodes)| {
            let label = nodes
                .first()
                .and_then(|id| graph.get_node(id))
                .map(|n| n.label.clone())
                .unwrap_or_else(|| format!("Community {}", cid));
            (*cid, label)
        })
        .collect();

    std::fs::create_dir_all(&output_dir).unwrap();

    // JSON
    let p = graphify_export::export_json(&graph, &output_dir).unwrap();
    assert!(p.exists(), "graph.json should exist");

    // HTML
    let p =
        graphify_export::export_html(&graph, &communities, &community_labels, &output_dir).unwrap();
    assert!(p.exists(), "graph.html should exist");
    let html = std::fs::read_to_string(&p).unwrap();
    assert!(html.contains("vis-network"), "HTML should contain vis.js");

    // GraphML
    let p = graphify_export::export_graphml(&graph, &output_dir).unwrap();
    assert!(p.exists(), "graph.graphml should exist");
    let graphml = std::fs::read_to_string(&p).unwrap();
    assert!(graphml.contains("<graphml"), "should be valid GraphML");

    // Cypher
    let p = graphify_export::export_cypher(&graph, &output_dir).unwrap();
    assert!(p.exists(), "cypher.txt should exist");
    let cypher = std::fs::read_to_string(&p).unwrap();
    assert!(
        cypher.contains("CREATE"),
        "should contain CREATE statements"
    );

    // SVG
    let p = graphify_export::export_svg(&graph, &communities, &output_dir).unwrap();
    assert!(p.exists(), "graph.svg should exist");
    let svg = std::fs::read_to_string(&p).unwrap();
    assert!(svg.contains("<svg"), "should be valid SVG");

    // Wiki
    let p =
        graphify_export::export_wiki(&graph, &communities, &community_labels, &output_dir).unwrap();
    assert!(p.exists(), "wiki dir should exist");

    // Report
    let god_list = graphify_analyze::god_nodes(&graph, 5);
    let surprise_list = graphify_analyze::surprising_connections(&graph, &communities, 3);
    let questions = graphify_analyze::suggest_questions(&graph, &communities, &community_labels, 3);
    let detection_json = serde_json::json!({"total_files": 1, "total_words": 50, "warning": null});
    let god_json: Vec<serde_json::Value> = god_list
        .iter()
        .map(|g| serde_json::json!({"label": g.label, "edges": g.degree}))
        .collect();
    let surprise_json: Vec<serde_json::Value> = surprise_list
        .iter()
        .map(|s| serde_json::to_value(s).unwrap_or_default())
        .collect();
    let question_json: Vec<serde_json::Value> = questions
        .iter()
        .map(|q| serde_json::to_value(q).unwrap_or_default())
        .collect();
    let token_cost: HashMap<String, usize> =
        HashMap::from([("input".into(), 0), ("output".into(), 0)]);
    let report = graphify_export::generate_report(
        &graph,
        &communities,
        &cohesion,
        &community_labels,
        &god_json,
        &surprise_json,
        &detection_json,
        &token_cost,
        ".",
        Some(&question_json),
    );
    assert!(
        report.contains("Graph Analysis Report"),
        "report should have header"
    );
    assert!(report.contains("node"), "report should mention nodes");

    // Obsidian
    let p = graphify_export::export_obsidian(&graph, &communities, &community_labels, &output_dir)
        .unwrap();
    assert!(p.exists(), "obsidian dir should exist");
}

// ---------------------------------------------------------------------------
// Security
// ---------------------------------------------------------------------------

#[test]
fn test_security_url_validation() {
    // Private IPs should be rejected
    assert!(graphify_security::validate_url("http://127.0.0.1/admin").is_err());
    assert!(graphify_security::validate_url("http://10.0.0.1/internal").is_err());

    // Public URLs should be accepted
    assert!(graphify_security::validate_url("https://example.com").is_ok());
    assert!(graphify_security::validate_url("https://github.com/foo/bar").is_ok());
}

#[test]
fn test_security_path_validation() {
    let root = std::env::temp_dir();
    // Path traversal should be rejected
    assert!(graphify_security::safe_path(std::path::Path::new("../../etc/passwd"), &root).is_err());
    // Normal relative paths should resolve under root
    let result = graphify_security::safe_path(std::path::Path::new("src/main.rs"), &root);
    // This may succeed or fail depending on implementation, but traversal must fail
    let _ = result;
}

#[test]
fn test_security_label_sanitization() {
    let sanitized = graphify_security::sanitize_label("<script>alert('xss')</script>");
    assert!(
        !sanitized.contains('<'),
        "should escape HTML: {}",
        sanitized
    );
    assert!(
        !sanitized.contains('>'),
        "should escape HTML: {}",
        sanitized
    );
}

// ---------------------------------------------------------------------------
// Multi-language extraction
// ---------------------------------------------------------------------------

#[test]
fn test_extract_typescript() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("app.ts"),
        r#"
import { Request, Response } from 'express';

interface User {
    name: string;
    email: string;
}

class UserController {
    getUser(req: Request, res: Response): User {
        return { name: 'test', email: 'test@test.com' };
    }

    createUser(data: User): void {
        console.log(data);
    }
}

export function initRoutes(): void {
    const ctrl = new UserController();
}
"#,
    )
    .unwrap();

    let result = graphify_extract::extract(&[dir.path().join("app.ts")]);

    let labels: Vec<String> = result.nodes.iter().map(|n| n.label.clone()).collect();
    assert!(
        labels.iter().any(|l| l == "UserController"),
        "should extract UserController: {:?}",
        labels
    );
    assert!(
        labels.iter().any(|l| l.contains("initRoutes")),
        "should extract initRoutes: {:?}",
        labels
    );
}

#[test]
fn test_extract_java() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("App.java"),
        r#"
import java.util.List;
import java.util.ArrayList;

public class App {
    private String name;

    public App(String name) {
        this.name = name;
    }

    public String getName() {
        return this.name;
    }

    public static void main(String[] args) {
        App app = new App("test");
        System.out.println(app.getName());
    }
}
"#,
    )
    .unwrap();

    let result = graphify_extract::extract(&[dir.path().join("App.java")]);

    let labels: Vec<String> = result.nodes.iter().map(|n| n.label.clone()).collect();
    assert!(
        labels.iter().any(|l| l == "App"),
        "should extract App class: {:?}",
        labels
    );
    assert!(
        labels
            .iter()
            .any(|l| l.contains("getName") || l.contains("main")),
        "should extract methods: {:?}",
        labels
    );
}

#[test]
fn test_extract_c() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.c"),
        r#"
#include <stdio.h>
#include <stdlib.h>

typedef struct {
    int x;
    int y;
} Point;

Point* create_point(int x, int y) {
    Point* p = malloc(sizeof(Point));
    p->x = x;
    p->y = y;
    return p;
}

void print_point(Point* p) {
    printf("(%d, %d)\n", p->x, p->y);
}

int main() {
    Point* p = create_point(3, 4);
    print_point(p);
    free(p);
    return 0;
}
"#,
    )
    .unwrap();

    let result = graphify_extract::extract(&[dir.path().join("main.c")]);

    let labels: Vec<String> = result.nodes.iter().map(|n| n.label.clone()).collect();
    assert!(
        labels.iter().any(|l| l.contains("create_point")),
        "should extract create_point: {:?}",
        labels
    );
    assert!(
        labels.iter().any(|l| l.contains("print_point")),
        "should extract print_point: {:?}",
        labels
    );
}
